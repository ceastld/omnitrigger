import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import {EventEmitter} from 'node:events';
import test from 'node:test';

const read=path=>readFileSync(new URL('../'+path,import.meta.url),'utf8');
const desktopSource=read('crates/omnitrigger-actions/runtime/desktop-api.js');
const factory=runInNewContext(desktopSource+'\ncreateDesktopApi');
const method='display.brightness_step';
const options={direction:'up',strategy:'physical_then_wmi'};
const plain=value=>JSON.parse(JSON.stringify(value));
const receipt=()=>({strategy:options.strategy,effectState:'unknown',physical:{members:[
  {index:0,status:'confirmed',previousRaw:40,requestedRaw:45,currentRaw:45,errorCode:null},
  {index:1,status:'set_unconfirmed',previousRaw:20,requestedRaw:25,currentRaw:null,errorCode:'brightness_unknown'},
  {index:2,status:'read_rejected',previousRaw:null,requestedRaw:null,currentRaw:null,errorCode:'brightness_unknown'},
],errorCode:null},wmi:null});
const nativeError=(code='cancelled',value=receipt())=>Object.assign(Error('PRIVATE_DEVICE'),
  {code,effectState:'unknown',method,brightnessReceipt:value});

test('同步和异步取消/超时保留已确认、未知、未读取的匿名成员且仅调用一次',async()=>{
  for(const asynchronous of [false,true])for(const code of ['cancelled','timeout']){
    let calls=0;
    const source=receipt();
    const cause=nativeError(code,source);
    const api=factory(()=>{calls++;if(asynchronous)return Promise.reject(cause);throw cause;});
    let failure;
    try{await api.desktop.stepBrightness(options);}catch(error){failure=error;}
    assert.equal(failure.code,code);
    assert.equal(failure.effectState,'unknown');
    assert.equal(failure.method,method);
    assert.deepEqual(plain(failure.brightnessReceipt),source);
    assert.notEqual(failure.brightnessReceipt,source);
    assert.notEqual(failure.brightnessReceipt.physical.members,source.physical.members);
    assert.ok(!failure.message.includes('PRIVATE'));
    assert.equal(calls,1);
    source.physical.members[0].currentRaw=99;
    assert.equal(failure.brightnessReceipt.physical.members[0].currentRaw,45);
  }
});

test('缺失、策略不匹配、附加标识、getter、错误effect均不能形成进度或重试证明',()=>{
  const invalid=[];
  invalid.push(undefined,null,{});
  const mismatched=receipt();mismatched.strategy='physical';invalid.push(mismatched);
  const leaking=receipt();leaking.physical.members[0].label='PRIVATE';invalid.push(leaking);
  const wrongEffect=receipt();wrongEffect.effectState='not_started';invalid.push(wrongEffect);
  const wrongIndex=receipt();wrongIndex.physical.members[1].index=0;invalid.push(wrongIndex);
  for(const value of invalid){
    const cause=nativeError();cause.brightnessReceipt=value;
    const api=factory(()=>{throw cause;});
    assert.throws(()=>api.desktop.stepBrightness(options),error=>{
      assert.equal(error.code,'cancelled');assert.equal(error.effectState,'unknown');
      assert.equal(Object.hasOwn(error,'brightnessReceipt'),false);return true;
    });
  }
  let getters=0;
  const cause=nativeError();
  Object.defineProperty(cause,'brightnessReceipt',{get(){getters++;throw Error('PRIVATE');}});
  assert.throws(()=>factory(()=>{throw cause;}).desktop.stepBrightness(options),error=>error.code==='cancelled'&&!Object.hasOwn(error,'brightnessReceipt'));
  assert.equal(getters,0);
  for(const code of ['brightness_failed','native_bridge_failed']){
    assert.throws(()=>factory(()=>{throw nativeError(code);}).desktop.stepBrightness(options),error=>error.code===code&&!Object.hasOwn(error,'brightnessReceipt'));
  }
  const wrongMethod=nativeError();wrongMethod.method='audio.volume_adjust';
  assert.throws(()=>factory(()=>{throw wrongMethod;}).desktop.stepBrightness(options),error=>error.code==='cancelled'&&!Object.hasOwn(error,'brightnessReceipt'));
});

test('未开始取消不携带任何已写回执',()=>{
  const cause=nativeError();cause.effectState='not_started';
  assert.throws(()=>factory(()=>{throw cause;}).desktop.stepBrightness(options),error=>{
    assert.equal(error.effectState,'not_started');assert.equal(error.brightnessReceipt,undefined);return true;
  });
});

function hostTransport(error){
  const source=read('crates/omnitrigger-actions/runtime/host.mjs');
  const start=source.indexOf('function nativeCall(method, params) {');
  const end=source.indexOf('\n}\n',start)+3;
  assert.ok(start>=0&&end>start);
  const sent=[];
  const context={Buffer,TextDecoder,AbortController,queueMicrotask,
    setTimeout:()=>({}),clearTimeout:()=>{},
    createConnection(){
      const socket=new EventEmitter();
      socket.setEncoding=()=>{};socket.setTimeout=()=>{};socket.destroy=()=>{};
      socket.write=frame=>{const request=JSON.parse(frame);sent.push(request);
        queueMicrotask(()=>socket.emit('data',JSON.stringify({id:request.id,ok:false,error})+'\n'));
      };
      queueMicrotask(()=>socket.emit('connect'));
      return socket;
    }};
  const invoke=runInNewContext(`
    const request={native:{port:1,token:'injected'},native_context_available:true};
    const controller=new AbortController();
    const nativeMethods=new Set(['display.brightness_step']);
    const nativeSockets=new Set();
    let nativeId=0,nativePending,nativePendingMethod;
    const checkCancelled=()=>{};
    const fault=(code,message)=>Object.assign(new Error(message),{code});
    ${source.slice(start,end)}
    nativeCall`,context);
  return {invoke,sent};
}

test('Node传输到公开SDK的注入完整路径保留回执；无socket或硬件',async()=>{
  for(const code of ['cancelled','timeout']){
    const value=receipt();
    const {invoke,sent}=hostTransport({code,message:'PRIVATE',method,effect_state:'unknown',brightness_receipt:value});
    await assert.rejects(factory(invoke).desktop.stepBrightness(options),error=>{
      assert.equal(error.code,code);assert.equal(error.effectState,'unknown');
      assert.deepEqual(plain(error.brightnessReceipt),value);return true;
    });
    assert.equal(sent.length,1);
    assert.equal(sent[0].method,method);
  }
});

test('Script桥的JSON异常到同一SDK的注入路径保留回执',()=>{
  const source=read('tools/js-engine-probe/src/sdk.js');
  const start=source.indexOf('function call(kind, args = {}, serialized) {');
  const end=source.indexOf('\nctx.fs =',start);
  assert.ok(start>=0&&end>start);
  for(const code of ['cancelled','timeout']){
    let calls=0;
    const value=receipt();
    const invoke=runInNewContext(`
      let calls=0;
      const encode=JSON.stringify,decode=JSON.parse;
      const localDesktopAllowed=true,legacyDesktopMethods={};
      const nativeCall=injectedNative;
      ${source.slice(start,end)}
      (method,params)=>call('native',{method,params})`,{
      injectedNative:()=>{calls++;return JSON.stringify({ok:false,error:{code,message:'PRIVATE',method,effect_state:'unknown',brightness_receipt:value}});}
    });
    assert.throws(()=>factory(invoke).desktop.stepBrightness(options),error=>{
      assert.equal(error.code,code);assert.equal(error.effectState,'unknown');
      assert.deepEqual(plain(error.brightnessReceipt),value);return true;
    });
    assert.equal(calls,1);
  }
});

test('Node和Script最外层结构化异常序列化均保留专用回执',()=>{
  const error=nativeError();
  const host=read('crates/omnitrigger-actions/runtime/host.mjs');
  const start=host.indexOf("    respond({ status: 'error', error: {");
  const end=host.indexOf('\n    } });',start)+'\n    } });'.length;
  assert.ok(start>=0&&end>start);
  let result;
  runInNewContext(host.slice(start,end),{error,errorDiagnostic:()=>'',respond:value=>{result=value;}});
  assert.deepEqual(plain(result.error.brightness_receipt),receipt());
  const main=read('tools/js-engine-probe/src/main.rs');
  const fields=main.match(/return JSON\.stringify\((\{code:error\.code[^\n]+?\})\);/)[1];
  result=runInNewContext('JSON.stringify('+fields+')',{error});
  assert.deepEqual(JSON.parse(result).brightness_receipt,receipt());
});


test('仅原始明确unknown的取消/超时错误可附回执；不把缺失或坏effect正规化成证明',()=>{
  for(const value of [undefined,null,'bad',false]){
    const cause=nativeError();if(value===undefined)delete cause.effectState;else cause.effectState=value;
    assert.throws(()=>factory(()=>{throw cause;}).desktop.stepBrightness(options),error=>error.code==='cancelled'&&error.effectState==='unknown'&&!Object.hasOwn(error,'brightnessReceipt'));
  }
  let getters=0;const cause=nativeError();Object.defineProperty(cause,'effectState',{get(){getters++;throw Error('PRIVATE');}});
  assert.throws(()=>factory(()=>{throw cause;}).desktop.stepBrightness(options),error=>error.code==='cancelled'&&!Object.hasOwn(error,'brightnessReceipt'));
  assert.equal(getters,0);
});

test('Node和Script wire缺method不转送专用回执，旧取消错误保持unknown',async()=>{
  const error={code:'cancelled',message:'PRIVATE',effect_state:'unknown',brightness_receipt:receipt()};
  const {invoke}=hostTransport(error);
  await assert.rejects(factory(invoke).desktop.stepBrightness(options),failure=>failure.code==='cancelled'&&failure.effectState==='unknown'&&!Object.hasOwn(failure,'brightnessReceipt'));
  const source=read('tools/js-engine-probe/src/sdk.js');const start=source.indexOf('function call(kind, args = {}, serialized) {'),end=source.indexOf('\nctx.fs =',start);
  const raw=runInNewContext('let calls=0; const encode=JSON.stringify,decode=JSON.parse; const localDesktopAllowed=true,legacyDesktopMethods={}; const nativeCall=injectedNative;'+source.slice(start,end)+"\n()=>call('native',{method:'display.brightness_step',params:{}})",{injectedNative:()=>JSON.stringify({ok:false,error})});
  assert.throws(raw,failure=>failure.code==='cancelled'&&failure.effectState==='unknown'&&!Object.hasOwn(failure,'brightnessReceipt'));
});

test('双运行时最外层仅投影合格自有数据字段；其它method/code/effect和getter不泄露进度',()=>{
  const host=read('crates/omnitrigger-actions/runtime/host.mjs');const start=host.indexOf("    respond({ status: 'error', error: {"),end=host.indexOf('\n    } });',start)+'\n    } });'.length;
  const rust=read('tools/js-engine-probe/src/main.rs');const literal=rust.match(/const FAULT_FIELDS: &str = "([\s\S]*?)";/)[1];
  const project=runInNewContext(JSON.parse('"'+literal.replace(/\\\r?\n[ \t]*/g,'')+'"'));
  const node=error=>{let result;runInNewContext(host.slice(start,end),{error,errorDiagnostic:()=>'',respond:value=>result=value});return plain(result.error);};
  const script=error=>JSON.parse(project(error));
  for(const change of [e=>e.method='audio.volume_adjust',e=>delete e.method,e=>e.code='brightness_failed',e=>e.effectState='not_started']){
    const error=nativeError();change(error);
    assert(!Object.hasOwn(node(error),'brightness_receipt'));assert(!Object.hasOwn(script(error),'brightness_receipt'));
  }
  let getters=0;const error=nativeError();Object.defineProperty(error,'brightnessReceipt',{get(){getters++;throw Error('PRIVATE');}});
  assert(!Object.hasOwn(node(error),'brightness_receipt'));assert(!Object.hasOwn(script(error),'brightness_receipt'));assert.equal(getters,0);
  const valid=nativeError();assert.deepEqual(node(valid).brightness_receipt,receipt());assert.deepEqual(script(valid).brightness_receipt,receipt());
});
