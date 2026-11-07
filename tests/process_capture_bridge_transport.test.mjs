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
const fault=(code,effect)=>error=>error.code===code&&error.effectState===effect
  &&error.method==='process.capture'&&!String(error.message).includes('PRIVATE');
test('capture总传输预算包含清理余量，不沿用5秒默认或零碎数据续时',async()=>{
  const h=harness(),{result,socket}=h.start(10000);
  assert.equal(socket.timeout,11500);assert.equal(h.timers[0].ms,11500);assert.equal(socket.encoding,undefined);
  h.send(socket,{ok:true,value:{code:0,stdout:'',stderr:''}});
  assert.equal((await result).code,0);assert.equal(h.timers[0].cleared,true);
  const max=h.start(60000);assert.equal(max.socket.timeout,61500);
  max.socket.expired();await assert.rejects(max.result,fault('timeout','unknown'));
});
test('严格UTF8支持跨包字符，不替换非法或未完成字节',async()=>{
  const h=harness(),first=h.start();
  const bytes=Buffer.from(JSON.stringify({id:first.socket.request.id,ok:true,value:{code:0,stdout:'😀',stderr:''}})+'\n');
  const emoji=bytes.indexOf(Buffer.from('😀'));
  first.socket.emit('data',bytes.subarray(0,emoji+2));first.socket.emit('data',bytes.subarray(emoji+2));
  assert.equal((await first.result).stdout,'😀');
  for(const bytes of [Buffer.from([0xff,0x0a]),Buffer.concat([Buffer.from('{"id":2,"ok":true,"value":null}\n'),Buffer.from([0xc2])])]) {
    const own=harness(),call=own.start();call.socket.emit('data',bytes);
    await assert.rejects(call.result,fault('invalid_response','unknown'));
  }
});
test('连接前后取消区别not_started和unknown且固定红删',async()=>{
  const before=harness();
  before.controller.abort(Object.assign(Error('PRIVATE'),{code:'cancelled'}));
  await assert.rejects(before.call('process.capture',{}),fault('cancelled','not_started'));
  assert.equal(before.sockets.length,0);
  const during=harness(),own=during.start();
  during.controller.abort(Object.assign(Error('PRIVATE'),{code:'cancelled'}));
  await assert.rejects(own.result,fault('cancelled','unknown'));assert.equal(own.socket.destroyed,true);
});
test('无原生桥明确拒绝，单调用并发仍拒绝，不回退Node旧run',async()=>{
  const missing=harness();delete missing.scope.request.native;
  await assert.rejects(missing.call('process.capture',{}),fault('native_bridge_unavailable','not_started'));
  assert.equal(missing.sockets.length,0);
  const h=harness(),own=h.start();
  await assert.rejects(h.call('process.capture',{}),fault('desktop_busy','not_started'));
  assert.equal(h.sockets.length,1);
  own.socket.emit('close');await assert.rejects(own.result,fault('desktop_connection','unknown'));
});
test('capture回执身份和形状严格，错误字段不注入权限',async()=>{
  for(const response of [
    {ok:true,value:{},extra:true},
    {ok:false,error:{code:'cancelled',message:'PRIVATE',effect_state:'impossible'}},
    {ok:false,error:{code:'cancelled',message:'PRIVATE',method:'other.method'}},
  ]) {
    const h=harness(),own=h.start();h.send(own.socket,response);
    await assert.rejects(own.result,fault('desktop_protocol','unknown'));
  }
  const h=harness(),own=h.start();
  h.send(own.socket,{ok:false,error:{code:'process_cleanup_failed',message:'PRIVATE',effect_state:'unknown',required_permissions:['invented']}});
  await assert.rejects(own.result,error=>fault('process_cleanup_failed','unknown')(error)&&!Object.hasOwn(error,'requiredPermissions'));
});
test('超过桥接回执容量不交付部分数据',async()=>{
  const h=harness(),own=h.start();
  own.socket.emit('data',Buffer.alloc(512*1024+2,0x61));
  await assert.rejects(own.result,fault('invalid_response','unknown'));
});
test('Node省略cwd传明确包目录，显式cwd保留，不继承App工作目录',async()=>{
  const factory=readFileSync(new URL('../crates/omnitrigger-actions/runtime/process-capture-api.js',import.meta.url),'utf8');
  const marker=host.indexOf('capture:createProcessCaptureApi(');
  assert.notEqual(marker,-1,'正式host尚未接入共享capture工厂');
  const start=marker+'capture:'.length;
  const end=host.indexOf(', true).capture',start)+', true).capture'.length;
  assert(end>start,'正式host的异步capture入口未完整定义');
  const calls=[];
  const capture=runInNewContext(factory+'\n('+host.slice(start,end)+');',{
    request:{package_root:'C:\\OwnedPackage'},
    nativeCall:(method,params)=>{calls.push({method,params:JSON.parse(JSON.stringify(params))});return Promise.resolve({code:0,stdout:'',stderr:''});},
  });
  await capture('C:\\Owned\\f.exe');
  await capture('C:\\Owned\\f.exe',[],{cwd:'C:\\Explicit'});
  assert.equal(calls[0].method,'process.capture');assert.equal(calls[0].params.cwd,'C:\\OwnedPackage');
  assert.equal(calls[1].params.cwd,'C:\\Explicit');
});
test('正式轻量SDK加载共同工厂后仍保持capture和旧run/start的不同契约',()=>{
  const runtime=new URL('../crates/omnitrigger-actions/runtime/',import.meta.url);
  const shared=['desktop-api.js','browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js']
    .map(name=>readFileSync(new URL(name,runtime),'utf8')).join('\n');
  const sdk=readFileSync(new URL('../tools/js-engine-probe/src/sdk.js',import.meta.url),'utf8');
  const calls=[];
  const scope={ctx:{},__hostCall:text=>{
    const request=JSON.parse(text);calls.push(request);
    return JSON.stringify({ok:true,value:request.kind==='capture'?{code:0,stdout:'',stderr:''}:{pid:7,exitCode:3,success:false}});
  }};
  runInNewContext(shared+sdk,scope);
  assert.equal(scope.ctx.process.capture('C:\\Owned\\f.exe').code,0);
  assert.equal(scope.ctx.process.run('legacy-program').exitCode,3);
  assert.equal(scope.ctx.process.start('legacy-program').pid,7);
  assert.equal(calls[0].kind,'capture');assert.equal(calls[0].timeout_ms,10000);
  assert.equal(calls[1].kind,'run');assert.equal(calls[1].timeout_ms,1000);
  assert.equal(calls[2].kind,'start');
});
