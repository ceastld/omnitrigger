// 执行正式host.mjs的nativeCall和正式SDK工厂；socket/timer为内存替身，不访问网络、UIA或进程。
import assert from 'node:assert/strict';
import {EventEmitter} from 'node:events';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
const host=readFileSync(new URL('../crates/omnitrigger-actions/runtime/host.mjs',import.meta.url),'utf8');
const factory=readFileSync(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const first=host.indexOf('const nativeMethods ='),last=host.indexOf(']);',first)+3;
const start=host.indexOf('function nativeCall('),end=host.indexOf('const desktopApis =',start);
assert(first>=0&&last>first&&start>=0&&end>start,'正式宿主入口边界发生变化，需更新测试接线');
function harness(){
  const sockets=[],timers=[];const controller=new AbortController();
  class Socket extends EventEmitter{
    setEncoding(value){this.encoding=value;}setTimeout(ms,callback){this.timeout=ms;this.expired=callback;}
    write(text){this.request=JSON.parse(text);}destroy(){this.destroyed=true;}
  }
  const scope={Buffer,TextDecoder,controller,nativeId:0,nativePending:undefined,nativePendingMethod:undefined,nativeSockets:new Set(),
    request:{native:{port:1,token:'synthetic'}},fault:(code,message)=>Object.assign(Error(message),{code}),
    checkCancelled:()=>{if(controller.signal.aborted)throw controller.signal.reason;},
    createConnection:()=>{const socket=new Socket();sockets.push(socket);return socket;},
    setTimeout:(callback,ms)=>{const timer={callback,ms};timers.push(timer);return timer;},clearTimeout:t=>{if(t)t.cleared=true;}};
  vm.createContext(scope);vm.runInContext(host.slice(first,last)+'\n'+host.slice(start,end)+'\n'+factory+'\nglobalThis.desktop=createDesktopApi(nativeCall).desktop;',scope);
  const begin=options=>{const result=scope.desktop.readSelectionText(options);const socket=sockets.at(-1);socket?.emit('connect');return{result,socket};};
  const send=(socket,response)=>socket.emit('data',JSON.stringify({id:socket.request.id,...response})+'\n');
  return{scope,sockets,timers,controller,begin,send};
}
const matches=code=>e=>e.code===code&&e.method==='selection.read_text'&&e.effectState==='not_started'&&!e.message.includes('PRIVATE');
test('实际nativeCall白名单允许显式truncate；空成功、wire快照及资源收尾一致',async()=>{
  const h=harness();const options={overflow:'truncate'};const own=h.begin(options);options.overflow='reject';
  assert.deepEqual(own.socket.request.params,{overflow:'truncate'});assert.equal(own.socket.request.method,'selection.read_text');
  h.send(own.socket,{ok:true,value:{text:'',truncated:false}});assert.equal((await own.result).text,'');
  assert(own.socket.destroyed);assert(h.timers[0].cleared);assert.equal(h.sockets.length,1);
});
test('实际nativeCall的provider错误经SDK仍为错误、不变空且只发送一次',async()=>{
  for(const code of ['selection_unavailable','target_changed','timeout']){
    const h=harness(),own=h.begin();h.send(own.socket,{ok:false,error:{code,message:'PRIVATE provider details',effect_state:'unknown'}});
    await assert.rejects(own.result,matches(code));assert(own.socket.destroyed);assert.equal(h.sockets.length,1);
  }
});
test('实际客户端在连接前和等待中取消时终止传输，不读取替代数据',async()=>{
  const before=harness();before.controller.abort(Object.assign(Error('PRIVATE'),{code:'cancelled'}));
  await assert.rejects(before.scope.desktop.readSelectionText(),matches('cancelled'));assert.equal(before.sockets.length,0);
  const during=harness(),own=during.begin();during.controller.abort(Object.assign(Error('PRIVATE'),{code:'cancelled'}));
  await assert.rejects(own.result,matches('cancelled'));assert(own.socket.destroyed);assert.equal(during.sockets.length,1);
});
test('读取的总传输定时器不会由部分数据续期，超时不交付空成功',async()=>{
  const h=harness(),own=h.begin();assert.equal(h.timers.length,1);const budget=h.timers[0];
  own.socket.emit('data','{"id":1');assert.equal(h.timers.length,1);budget.callback();
  await assert.rejects(own.result,matches('timeout'));assert(own.socket.destroyed);
});
test('客户端拒绝并发、错误身份、断连及无桥接，均不补发读取',async()=>{
  const h=harness(),own=h.begin();await assert.rejects(h.scope.desktop.readSelectionText(),matches('desktop_busy'));
  assert.equal(h.sockets.length,1);own.socket.emit('data','{"id":999,"ok":true,"value":{"text":"","truncated":false}}\n');
  await assert.rejects(own.result,matches('desktop_protocol'));
  const closed=harness(),pending=closed.begin();pending.socket.emit('close');await assert.rejects(pending.result,matches('desktop_connection'));
  const missing=harness();delete missing.scope.request.native;
  await assert.rejects(missing.scope.desktop.readSelectionText(),matches('native_bridge_unavailable'));assert.equal(missing.sockets.length,0);
});
test('Node桥既有128配额保持独立于轻量32配额，空成功不重置计数',async()=>{
  const h=harness();h.scope.nativeId=127;const own=h.begin();assert.equal(own.socket.request.id,128);
  h.send(own.socket,{ok:true,value:{text:'',truncated:false}});await own.result;
  await assert.rejects(h.scope.desktop.readSelectionText(),matches('desktop_call_limit'));assert.equal(h.sockets.length,1);
});
