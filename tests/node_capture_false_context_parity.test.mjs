// 仅读正式宿主源码并评估客户端函数；所有socket/timer都是内存替身，不启动网络或进程。
import assert from 'node:assert/strict';
import {EventEmitter} from 'node:events';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
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

test('false上下文的实际capture Host可收到唯一请求',async()=>{
  const h=harness();h.scope.request.native_context_available=false;
  const {result,socket}=h.start(1379);
  assert.equal(h.sockets.length,1);assert.equal(socket.timeout,2879);
  assert.equal(socket.request.method,'process.capture');
  assert.deepEqual(JSON.parse(JSON.stringify(socket.request.params)),{program:'C:\\Owned\\f.exe',args:[],timeout_ms:1379,allow_non_zero:false});
  h.send(socket,{ok:true,value:{code:0,stdout:'synthetic 中文😀',stderr:''}});
  assert.equal((await result).stdout,'synthetic 中文😀');
});
test('false上下文Host拒绝不转为本地启动或重试',async()=>{
  const h=harness();h.scope.request.native_context_available=false;
  const {result,socket}=h.start();
  h.send(socket,{ok:false,error:{code:'desktop_unavailable',message:'synthetic',method:'process.capture',effect_state:'not_started'}});
  await assert.rejects(result,e=>e.code==='desktop_unavailable'&&e.effectState==='not_started'&&e.method==='process.capture');
  assert.equal(h.sockets.length,1);
});
test('false上下文缺桥仍在提交前拒绝',async()=>{
  const h=harness();h.scope.request.native_context_available=false;delete h.scope.request.native;
  await assert.rejects(h.call('process.capture',{}),e=>e.code==='native_bridge_unavailable'&&e.effectState==='not_started');
  assert.equal(h.sockets.length,0);
});
test('无目标opening也只经实际Host判断，不提升false或本地回退',async()=>{
  for(const method of ['system.open','system.open_file_in_parent']){
    const h=harness();h.scope.request.native_context_available=false;h.scope.nativeMethods.add(method);
    const result=h.call(method,{target:method==='system.open'?'https://synthetic.invalid/':'C:/Owned/never.txt'});
    const socket=h.sockets[0];socket.emit('connect');h.send(socket,{ok:true,value:{submitted:true}});
    assert.equal((await result).submitted,true);
    assert.equal(h.sockets.length,1);assert.equal(socket.request.method,method);
    assert.equal(h.scope.request.native_context_available,false);
  }
});
