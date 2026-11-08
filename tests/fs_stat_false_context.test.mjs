// 实际SDK + 已有通用Desktop工厂的VM；回调仅返回合成JSON，绝不调用Native/OS。
import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
const root=new URL('../',import.meta.url);
const read=path=>readFileSync(new URL(path,root),'utf8');
const sdk=read('tools/js-engine-probe/src/sdk.js');
const desktop=read('crates/omnitrigger-actions/runtime/desktop-api.js');
const connection={port:12345,token:'a'.repeat(64)};
function boot(context,allowed,response,source=sdk){
  const calls=[];
  const ctx={input:{},__nativeContext:context,...(allowed===undefined?{}:{__localDesktopAllowed:allowed})};
  const sandbox=vm.createContext({ctx,__hostCall:raw=>{
    const frame=JSON.parse(raw);calls.push(frame);
    let value=response?.(frame);
    if(value===undefined)value=frame.kind==='native'&&frame.method==='clipboard.read'||frame.kind==='read_clipboard'?'synthetic text':
      frame.kind==='native'&&frame.method.startsWith('system.open')?{submitted:true}:null;
    return JSON.stringify(value?.syntheticError?{ok:false,error:value.syntheticError}:{ok:true,value});
  }},{codeGeneration:{strings:false,wasm:false}});
  const otherFactories=`function createProcessCaptureApi(){return {capture(){throw Error('not under test')}};}
    function createStateApi(){return {};};function createActionsApi(){return {};}
    function createBrowserApi(){return {};};function createClipboardHistoryApi(){return {};}`;
  new vm.Script(desktop+'\n'+otherFactories+'\n'+source).runInContext(sandbox,{timeout:1000});
  return {calls,metadata:context,run:code=>new vm.Script(code).runInContext(sandbox,{timeout:1000})};
}

import {EventEmitter} from 'node:events';
const runInNewContext=vm.runInNewContext;
const host=readFileSync(new URL('../crates/omnitrigger-actions/runtime/host.mjs',import.meta.url),'utf8');
const body=host.slice(host.indexOf('function nativeCall('),host.indexOf('const desktopApis ='));
function harness() {
  const sockets=[],timers=[];
  class Socket extends EventEmitter {
    setEncoding(value){this.encoding=value;}
    setTimeout(ms,handler){this.timeout=ms;this.expired=handler;}
    write(text){this.request=JSON.parse(text);}
    destroy(){this.destroyed=true;}
  }
  const controller=new AbortController();
  const scope={Buffer,TextDecoder,Promise,Set,controller,nativeMethods:new Set(['process.capture']),
    nativeSockets:new Set(),nativeId:0,nativePending:undefined,nativePendingMethod:undefined,
    request:{native:{port:1,token:'synthetic'},package_root:'C:\\OwnedPackage'},
    fault:(code,message)=>Object.assign(Error(message),{code}),
    checkCancelled:()=>{if(controller.signal.aborted)throw controller.signal.reason;},
    createConnection:()=>{const socket=new Socket();sockets.push(socket);return socket;},
    setTimeout:(callback,ms)=>{const timer={callback,ms};timers.push(timer);return timer;},
    clearTimeout:timer=>{if(timer)timer.cleared=true;},
  };
  const call=runInNewContext(body+'\nnativeCall;',scope);
  const start=(timeout_ms=10000)=>{
    const result=call('process.capture',{program:'C:\\Owned\\f.exe',args:[],timeout_ms,allow_non_zero:false});
    const socket=sockets.at(-1);socket?.emit('connect');
    return {result,socket};
  };
  const send=(socket,value)=>socket.emit('data',Buffer.from(JSON.stringify({id:socket.request.id,...value})+'\n'));
  return {call,start,send,scope,sockets,timers,controller};
}


const plain=value=>JSON.parse(JSON.stringify(value));
const target='C:/OwnedMissing/space & 中文😀.txt';
function nodeFs() {
  const h=harness();h.scope.request.native_context_available=false;h.scope.nativeMethods.add('fs.stat');
  const factory=runInNewContext(desktop+'\ncreateDesktopApi;',{});
  return {...h,fs:factory(h.call).fs};
}
for(const kind of ['file','directory','missing','other']) {
  test(`Script false/stat ${kind}唯一native调用、原路径、false不变`,()=>{
    const h=boot({connection,native_context_available:false},false,()=>({kind}));
    assert.deepEqual(plain(h.run(`ctx.fs.stat(${JSON.stringify(target)})`)),{kind});
    assert.equal(h.calls.length,1);assert.equal(h.calls[0].kind,'native');
    assert.equal(h.calls[0].method,'fs.stat');assert.deepEqual(h.calls[0].params,{path:target});
    assert.equal(h.metadata.native_context_available,false);
  });
  test(`Node false/stat ${kind}唯一内存socket，外层工厂合同不变`,async()=>{
    const h=nodeFs(),result=h.fs.stat(target),socket=h.sockets[0];socket.emit('connect');
    h.send(socket,{ok:true,value:{kind}});
    assert.deepEqual(plain(await result),{kind});assert.equal(h.sockets.length,1);
    assert.equal(socket.request.method,'fs.stat');assert.deepEqual(socket.request.params,{path:target});
    assert.equal(h.scope.request.native_context_available,false);
  });
}
for(const code of ['fs_unavailable','path_unavailable','access_denied','timeout']) {
  test(`Script false/stat ${code}保持只读错误合同且无回退`,()=>{
    const h=boot({connection,native_context_available:false},false,()=>({syntheticError:{code,effect_state:'not_started',method:'fs.stat',message:'synthetic'}}));
    assert.throws(()=>h.run(`ctx.fs.stat(${JSON.stringify(target)})`),e=>e.code===code&&e.effectState==='not_started'&&e.method==='fs.stat');
    assert.equal(h.calls.length,1);assert.equal(h.calls[0].kind,'native');
    assert.equal(h.metadata.native_context_available,false);
  });
  test(`Node false/stat ${code}唯一派发并保持not_started`,async()=>{
    const h=nodeFs(),result=h.fs.stat(target),socket=h.sockets[0];socket.emit('connect');
    h.send(socket,{ok:false,error:{code,effect_state:'not_started',method:'fs.stat',message:'synthetic'}});
    await assert.rejects(result,e=>e.code===code&&e.effectState==='not_started'&&e.method==='fs.stat');
    assert.equal(h.sockets.length,1);assert.equal(h.scope.request.native_context_available,false);
  });
}
test('两SDK缺桥、建桥失败不派发；坏回执统一只读fs_error',async()=>{
  for(const context of [null,{native_context_available:false},{error:{code:'native_bridge_unavailable',effect_state:'not_started'}}]) {
    const h=boot(context,false);
    assert.throws(()=>h.run(`ctx.fs.stat(${JSON.stringify(target)})`),e=>e.code==='fs_unavailable'&&e.effectState==='not_started');
    assert.equal(h.calls.length,0);
  }
  for(const construction of [false,true]) {
    const h=nodeFs();delete h.scope.request.native;
    if(construction)h.scope.request.native_error={code:'native_bridge_unavailable'};
    await assert.rejects(h.fs.stat(target),e=>e.code==='fs_unavailable'&&e.effectState==='not_started');
    assert.equal(h.sockets.length,0);
  }
  const s=boot({connection,native_context_available:false},false,()=>({kind:'file',extra:true}));
  assert.throws(()=>s.run(`ctx.fs.stat(${JSON.stringify(target)})`),e=>e.code==='fs_error'&&e.effectState==='not_started');
  assert.equal(s.calls.length,1);
  const h=nodeFs(),result=h.fs.stat(target),socket=h.sockets[0];socket.emit('connect');
  h.send(socket,{ok:true,value:{kind:'file',extra:true}});
  await assert.rejects(result,e=>e.code==='fs_error'&&e.effectState==='not_started');assert.equal(h.sockets.length,1);
});
test('路径/云盘守卫先于派发，read_directory、Window、Clipboard仍拒绝false',async()=>{
  for(const path of ['relative.txt','C:/OneDrive/no.txt','C:/Dropbox/no.txt','C:/iCloudDrive/no.txt']) {
    const s=boot({connection,native_context_available:false},false);
    assert.throws(()=>s.run(`ctx.fs.stat(${JSON.stringify(path)})`),e=>e.effectState==='not_started');assert.equal(s.calls.length,0);
    const n=nodeFs();assert.throws(()=>n.fs.stat(path),e=>e.effectState==='not_started');assert.equal(n.sockets.length,0);
  }
  for(const method of ['fs.read_directory','window.geometry','clipboard.read','clipboard.write']) {
    const s=boot({connection,native_context_available:false},false);
    assert.throws(()=>s.run(`desktopCall(${JSON.stringify(method)},{})`),e=>e.effectState==='not_started');assert.equal(s.calls.length,0);
    const n=nodeFs();n.scope.nativeMethods.add(method);
    await assert.rejects(n.call(method,{}),e=>e.effectState==='not_started');assert.equal(n.sockets.length,0);
  }
});
