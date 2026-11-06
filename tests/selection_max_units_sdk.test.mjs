// 真实工厂、真实同步SDK和真实Node nativeCall；记录端口，不读取UIA、Clipboard或网络。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {EventEmitter} from 'node:events';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
const runtime=new URL('../crates/omnitrigger-actions/runtime/',import.meta.url);
const desktop=readFileSync(new URL('desktop-api.js',runtime),'utf8');
const support=['browser-api.js','clipboard-history-api.js','process-capture-api.js'].map(name=>readFileSync(new URL(name,runtime),'utf8')).join('\n');
const sdk=readFileSync(new URL('../tools/js-engine-probe/src/sdk.js',import.meta.url),'utf8');
const host=readFileSync(new URL('host.mjs',runtime),'utf8');
const createApi=runInNewContext(desktop+';createDesktopApi');
const plain=value=>JSON.parse(JSON.stringify(value));
const matches=code=>error=>error.code===code&&error.method==='selection.read_text'&&error.effectState==='not_started'&&!error.message.includes('PRIVATE');
test('真实工厂默认wire不变，显式整数界限只传给同一个 selection.read_text',async()=>{
  for(const asynchronous of [false,true]) {
    const calls=[],api=createApi((method,params)=>{calls.push([method,plain(params)]);const value={text:'',truncated:false};return asynchronous?Promise.resolve(value):value;}).desktop;
    await api.readSelectionText();await api.readSelectionText({overflow:'truncate'});
    for(const maxUtf16Units of [1,128,129,2048,8192])await api.readSelectionText({maxUtf16Units});
    assert.deepEqual(calls,[['selection.read_text',{}],['selection.read_text',{overflow:'truncate'}],...[1,128,129,2048,8192].map(limit=>['selection.read_text',{max_utf16_units:limit}])]);
  }
});
test('0、小数、超过8192或65536等非法选项在任何宿主请求前拒绝',()=>{
  let calls=0;const api=createApi(()=>{calls++;return{text:'',truncated:false};}).desktop;
  for(const maxUtf16Units of [0,-1,1.5,8193,65536,NaN,Infinity,'8192',null,[],{},true,1n])assert.throws(()=>api.readSelectionText({maxUtf16Units}),matches('invalid_input'));
  assert.throws(()=>api.readSelectionText({maxUtf16Units:8192,extra:true}),matches('invalid_input'));
  assert.throws(()=>api.readSelectionText({get maxUtf16Units(){throw Error('PRIVATE');}}),matches('invalid_input'));
  assert.equal(calls,0);
});
test('8192单元最坏JSON、完整emoji与空文本均有界；不修复断裂代理项',async()=>{
  for(const asynchronous of [false,true])for(const text of ['\u0001'.repeat(8192),'字'.repeat(8192),'😀'.repeat(4096),'']) {
    const api=createApi(()=>asynchronous?Promise.resolve({text,truncated:false}):{text,truncated:false}).desktop;
    const result=await api.readSelectionText({maxUtf16Units:8192});
    assert.equal(result.text,text);assert(Object.isFrozen(result));assert(Buffer.byteLength(JSON.stringify(result),'utf8')<64*1024);
  }
  for(const value of [{text:'A'.repeat(8193),truncated:false},{text:'short',truncated:true},{text:'A'.repeat(8191)+'\ud800',truncated:true},{text:'\0',truncated:false},{text:'ok',truncated:false,max_utf16_units:8192}])
    assert.throws(()=>createApi(()=>value).desktop.readSelectionText({maxUtf16Units:8192,overflow:'truncate'}),matches('invalid_response'));
  assert.deepEqual(plain(createApi(()=>({text:'字'.repeat(2048),truncated:true})).desktop.readSelectionText({maxUtf16Units:2048,overflow:'truncate'})),{text:'字'.repeat(2048),truncated:true});
  assert.throws(()=>createApi(()=>({text:'A'.repeat(128),truncated:true})).desktop.readSelectionText({maxUtf16Units:8192,overflow:'truncate'}),matches('invalid_response'));
});
test('实际同步SDK转发新界限并返回同样快照，仍不补读取或Clipboard',()=>{
  const calls=[];const raw='\u0001'.repeat(8192);
  const scope={ctx:{__nativeContext:{connection:{port:1,token:'synthetic'}}},__hostCall:serialized=>{
    calls.push(JSON.parse(serialized));return JSON.stringify({ok:true,value:{text:raw,truncated:true}});
  }};
  runInNewContext(desktop+support+sdk,scope);
  const value=scope.ctx.desktop.readSelectionText({maxUtf16Units:8192,overflow:'truncate'});
  assert.equal(value.text,raw);assert.equal(value.truncated,true);assert.equal(typeof value.then,'undefined');
  assert.deepEqual(calls.map(call=>[call.kind,call.method,call.params]),[['native','selection.read_text',{overflow:'truncate',max_utf16_units:8192}]]);
});
test('实际Node nativeCall传递已固定的上限，49KiB回执不突破512KiB传输限制',async()=>{
  const sockets=[],timers=[],controller=new AbortController();
  class Socket extends EventEmitter {
    setEncoding(){}setTimeout(ms,callback){this.ms=ms;this.expired=callback;}
    write(frame){this.frame=JSON.parse(frame);}destroy(){this.closed=true;}
  }
  const methodStart=host.indexOf('const nativeMethods ='),methodEnd=host.indexOf(']);',methodStart)+3;
  const start=host.indexOf('function nativeCall('),end=host.indexOf('const desktopApis =',start);
  const scope={Buffer,TextDecoder,controller,nativeId:0,nativePending:undefined,nativePendingMethod:undefined,nativeSockets:new Set(),request:{native:{port:1,token:'synthetic'}},
    checkCancelled(){},fault:(code,message)=>Object.assign(Error(message),{code}),
    createConnection(){const socket=new Socket();sockets.push(socket);return socket;},setTimeout(callback,ms){const timer={callback,ms};timers.push(timer);return timer;},clearTimeout(timer){if(timer)timer.cleared=true;}};
  const call=runInNewContext(host.slice(methodStart,methodEnd)+'\n'+host.slice(start,end)+'\nnativeCall;',scope);
  const api=createApi(call).desktop,options={maxUtf16Units:8192};
  const pending=api.readSelectionText(options);options.maxUtf16Units=1;const socket=sockets[0];socket.emit('connect');
  assert.deepEqual(socket.frame.params,{max_utf16_units:8192});assert.equal(socket.ms,5000);
  const value={text:'\u0001'.repeat(8192),truncated:false};
  const frame=JSON.stringify({id:socket.frame.id,ok:true,value})+'\n';assert(Buffer.byteLength(frame)<64*1024);
  socket.emit('data',frame.slice(0,40000));socket.emit('data',frame.slice(40000));
  assert.equal((await pending).text,value.text);assert(socket.closed&&timers[0].cleared);assert.equal(sockets.length,1);
});
test('更大上限不改变平台超长/取消/期限等错误，也不吞成空成功',async()=>{
  for(const code of ['selection_too_long','selection_empty','selection_changed','target_changed','timeout','cancelled','selection_helper_cleanup_failed']) {
    let count=0;const api=createApi(()=>{count++;return Promise.reject(Object.assign(Error('PRIVATE'),{code}));}).desktop;
    await assert.rejects(api.readSelectionText({maxUtf16Units:8192}),matches(code));assert.equal(count,1);
  }
});
