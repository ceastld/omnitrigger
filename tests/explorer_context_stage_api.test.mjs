// 精确opt-in参数与真实共享工厂；仅内存bridge，未执行COM/Explorer/Clipboard。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import vm from 'node:vm';
import {EventEmitter} from 'node:events';
import test from 'node:test';
const repo=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const read=path=>readFileSync(resolve(repo,path),'utf8');
const source=read('crates/omnitrigger-actions/runtime/desktop-api.js');
const create=vm.runInThisContext('(()=>{'+source+';return createDesktopApi;})()');
const host=read('crates/omnitrigger-actions/runtime/host.mjs');
const at=host.indexOf('const asyncApi =');
const asyncApi=vm.runInThisContext('(()=>{'+host.slice(at,host.indexOf('\n',at))+';return asyncApi;})()');
const method='explorer.context';
const snapshot=()=>({currentDirectory:'D:\\fixture',selectedPaths:['D:\\fixture\\a.txt']});
const nativeFault=(code,effectState='not_started',reportedMethod=method)=>Object.assign(Error('PRIVATE Native detail'),{code,method:reportedMethod,effectState});
const invalid=error=>error.code==='invalid_input'&&error.method===method&&error.effectState==='not_started';
function boot(asynchronous,reply=snapshot()){
  const calls=[];const invoke=(name,params)=>{calls.push([name,params]);
    if(reply instanceof Error){if(asynchronous)return Promise.reject(reply);throw reply;}
    return asynchronous?Promise.resolve(reply):reply;
  };
  const desktop=create(invoke).desktop;
  return {calls,api:asynchronous?asyncApi(desktop):desktop};
}
test('默认零参数仍发{}，显式true才发snake-case；同步与Promise成功快照形状不变',async()=>{
  for(const asynchronous of [false,true]){
    const native=snapshot(),h=boot(asynchronous,native);
    for(const args of [[],[{selectionFailureStage:true}]]){
      const value=h.api.explorerContext(...args);assert.equal(typeof value.then==='function',asynchronous);
      const result=await value;assert.deepEqual(result,snapshot());assert(Object.isFrozen(result)&&Object.isFrozen(result.selectedPaths));
    }
    assert.deepEqual(h.calls,[[method,{}],[method,{selection_failure_stage:true}]]);
    assert.deepEqual(native,snapshot());
  }
});
test('空对象/false/extra/getter等仍在Native前拒绝，检查不执行getter',async()=>{
  let getters=0;const getter={get selectionFailureStage(){getters++;throw Error('GETTER MUST NOT RUN');}};
  const extra={selectionFailureStage:true};Object.defineProperty(extra,Symbol('extra'),{get(){getters++;throw Error('GETTER MUST NOT RUN');}});
  const inherited=Object.create({get selectionFailureStage(){getters++;throw Error('GETTER MUST NOT RUN');}});
  for(const asynchronous of [false,true]){
    const h=boot(asynchronous);
    for(const args of [[{}],[false],[true],[null],[undefined],[[]],[{selectionFailureStage:false}],
      [{selectionFailureStage:true,other:1}],[{selection_failure_stage:true}],[getter],[extra],[inherited],[{selectionFailureStage:true},{}]]){
      if(asynchronous)await assert.rejects(h.api.explorerContext(...args),invalid);
      else assert.throws(()=>h.api.explorerContext(...args),invalid);
    }
    assert.equal(h.calls.length,0);
  }
  assert.equal(getters,0);
});
test('新码只转交Native的method/effect，不从true或旧码生成目录证明',async()=>{
  for(const asynchronous of [false,true])for(const [reportedMethod,effectState] of [[method,'not_started'],['other.method','unknown'],[undefined,undefined]]){
    const cause=nativeFault('selection_unavailable_after_directory');cause.method=reportedMethod;cause.effectState=effectState;
    const h=boot(asynchronous,cause);let error;
    try{await h.api.explorerContext({selectionFailureStage:true});}catch(value){error=value;}
    assert.equal(error.code,'selection_unavailable_after_directory');assert.equal(error.method,reportedMethod);assert.equal(error.effectState,effectState);
    assert(!error.message.includes('PRIVATE'));assert.equal(h.calls.length,1);
  }
});
test('其他读取/目标/取消错误仍抛出，不吞掉、不变成新码或成功空快照',async()=>{
  for(const asynchronous of [false,true])for(const code of ['selection_unavailable','cancelled','timeout','target_changed','ambiguous_target','selection_changed','unsupported_target','unsupported_item','too_large','invalid_response']){
    const h=boot(asynchronous,nativeFault(code));let error;
    try{await h.api.explorerContext({selectionFailureStage:true});}catch(value){error=value;}
    assert.equal(error.code,code);assert.equal(error.method,method);assert.equal(error.effectState,'not_started');assert.equal(h.calls.length,1);
  }
});
test('显式参数固定为true快照；旧Host拒绝opt-in时不隐式零参数重试',async()=>{
  const options={selectionFailureStage:true},h=boot(true);
  const pending=h.api.explorerContext(options);options.selectionFailureStage=false;
  await pending;assert.deepEqual(h.calls,[[method,{selection_failure_stage:true}]]);
  for(const asynchronous of [false,true]){
    const old=boot(asynchronous,nativeFault('invalid_input'));
    if(asynchronous)await assert.rejects(old.api.explorerContext({selectionFailureStage:true}),invalid);
    else assert.throws(()=>old.api.explorerContext({selectionFailureStage:true}),invalid);
    assert.deepEqual(old.calls,[[method,{selection_failure_stage:true}]]);
  }
});

test('真实Node nativeCall仅在显式true的新码保留method/effect；默认与其他错误保持旧正规化',async()=>{
  const start=host.indexOf('const nativeMethods =');
  const methods=host.slice(start,host.indexOf(']);',start)+3);
  const native=host.slice(host.indexOf('function nativeCall('),host.indexOf('const desktopApis ='));
  for(const [opt,code,reportedMethod,effectState] of [
    [true,'selection_unavailable_after_directory',method,'not_started'],
    [true,'selection_unavailable_after_directory','other.method','unknown'],
    [true,'selection_unavailable_after_directory',undefined,undefined],
    [false,'selection_unavailable',method,'not_started'],
    [true,'selection_changed','other.method','unknown'],
  ]){
    let socket;class Socket extends EventEmitter {
      setEncoding(){}setTimeout(){}write(text){this.request=JSON.parse(text);}destroy(){}
    }
    const controller=new AbortController();const scope={Buffer,TextDecoder,controller,nativeSockets:new Set(),nativeId:0,nativePending:undefined,nativePendingMethod:undefined,
      request:{native:{port:1,token:'synthetic'},native_context_available:true},
      fault:(code,message)=>Object.assign(Error(message),{code}),checkCancelled:()=>{},
      createConnection:()=>{socket=new Socket();return socket;},setTimeout:()=>({}),clearTimeout:()=>{}};
    const sandbox=vm.createContext(scope);
    vm.runInContext(source+'\n'+methods+'\n'+native+'\n'+host.slice(at,host.indexOf('\n',at))+'\nconst api=asyncApi(createDesktopApi(nativeCall).desktop);',sandbox);
    const pending=vm.runInContext('api.explorerContext('+(opt?'{selectionFailureStage:true}':'')+')',sandbox);
    socket.emit('connect');socket.emit('data',JSON.stringify({id:socket.request.id,ok:false,error:{code,message:'PRIVATE',method:reportedMethod,effect_state:effectState}})+'\n');
    let error;try{await pending;}catch(value){error=value;}
    assert.equal(error.code,code);
    assert.equal(error.method,opt&&code==='selection_unavailable_after_directory'?reportedMethod:method);
    assert.equal(error.effectState,opt&&code==='selection_unavailable_after_directory'?effectState:'not_started');
    assert.deepEqual(socket.request.params,opt?{selection_failure_stage:true}:{});assert.equal(scope.nativeId,1);
  }
});
