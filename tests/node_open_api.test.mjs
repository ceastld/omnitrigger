// 只评估正式客户端函数；socket、计时器与原生回执全为内存替身，不打开任何目标。
import assert from 'node:assert/strict';
import {EventEmitter} from 'node:events';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
const host=readFileSync(new URL('../crates/omnitrigger-actions/runtime/host.mjs', import.meta.url),'utf8');
const body=host.slice(host.indexOf('function nativeCall('),host.indexOf('const desktopApis ='));
const plain=value=>JSON.parse(JSON.stringify(value));
function harness(){
  const sockets=[],timers=[];
  class Socket extends EventEmitter{
    setEncoding(value){this.encoding=value;}
    setTimeout(ms,handler){this.timeout=ms;this.expire=handler;}
    write(text){this.request=JSON.parse(text);}
    destroy(){this.destroyed=true;}
  }
  const controller=new AbortController();
  const scope={Buffer,TextDecoder,Promise,Set,controller,nativeMethods:new Set(['system.open']),
    nativeSockets:new Set(),nativeId:0,nativePending:undefined,nativePendingMethod:undefined,
    request:{native:{port:1,token:'synthetic'}},
    fault:(code,message)=>Object.assign(Error(message),{code}),
    checkCancelled:()=>{if(controller.signal.aborted)throw controller.signal.reason;},
    createConnection:()=>{const socket=new Socket();sockets.push(socket);return socket;},
    setTimeout:(fn,ms)=>{const timer={fn,ms};timers.push(timer);return timer;},
    clearTimeout:timer=>{if(timer)timer.cleared=true;},
  };
  const open=runInNewContext(body+'\nopenTarget;',scope);
  function start(target='https://example.invalid/',connect=true){
    const result=open(target);result.catch(()=>{});
    const socket=sockets.at(-1);
    if(connect)socket.emit('connect');
    return{result,socket};
  }
  const send=(socket,value)=>socket.emit('data',JSON.stringify({id:socket.request.id,...value})+'\n');
  return{open,start,send,scope,controller,sockets,timers};
}
function fault(code,effect){return error=>{
  assert.equal(error.code,code);assert.equal(error.effectState,effect);
  assert.equal(error.method,'system.open');assert(!error.message.includes('PRIVATE'));
  assert.equal(error.requiredPermissions,undefined);return true;
};}
test('目标原样经过现有桥接，只返回严格系统接纳回执',async()=>{
  for(const target of ['https://example.invalid/?q=%E4%B8%AD','mailto:p@example.invalid','about:blank','C:/Owned/space & 中.txt']){
    const h=harness(),own=h.start(target);
    assert.deepEqual(plain(own.socket.request.params),{target});
    assert.equal(own.socket.request.method,'system.open');
    h.send(own.socket,{ok:true,value:{submitted:true}});
    assert.deepEqual(plain(await own.result),{submitted:true});
    assert.equal(h.sockets.length,1);
  }
});
test('单字符串/Unicode/长度契约在连接前校验，不推断参数或选项',async()=>{
  for(const args of [[],[42],[''],[' about:blank'],['about:blank\0'],['\ud800'],['x'.repeat(32767)],['about:blank',{}]]){
    const h=harness();await assert.rejects(h.open(...args),fault('invalid_input','not_started'));
    assert.equal(h.sockets.length,0);
  }
  const h=harness(),own=h.start('javascript:PRIVATE');
  h.send(own.socket,{ok:false,error:{code:'invalid_input',message:'PRIVATE',method:'system.open',effect_state:'not_started'}});
  await assert.rejects(own.result,fault('invalid_input','not_started'));
});
test('缺桥接与预取消不连接；没有run/start或其它回退',async()=>{
  const missing=harness();delete missing.scope.request.native;
  await assert.rejects(missing.open('about:blank'),fault('native_bridge_unavailable','not_started'));
  assert.equal(missing.sockets.length,0);
  const stopped=harness();stopped.controller.abort(Object.assign(Error('PRIVATE'),{code:'cancelled'}));
  await assert.rejects(stopped.open('about:blank'),fault('cancelled','not_started'));
  assert.equal(stopped.sockets.length,0);
});
test('结果、帧与错误归属严格校验，不泄漏正文或权限字段',async()=>{
  for(const response of [
    {ok:true,value:null},{ok:true,value:{submitted:false}},{ok:true,value:{submitted:true,extra:1}},
    {ok:true,value:{submitted:true},extra:1},
    {ok:false,error:{code:'open_failed',message:'PRIVATE',method:'other.method'}},
    {ok:false,error:{code:'open_failed',message:'PRIVATE',effect_state:'impossible'}},
  ]){
    const h=harness(),own=h.start();h.send(own.socket,response);
    await assert.rejects(own.result,fault('invalid_response','unknown'));
  }
  const h=harness(),own=h.start();
  h.send(own.socket,{ok:false,error:{code:'open_failed',message:'PRIVATE',effect_state:'unknown',required_permissions:['invented']}});
  await assert.rejects(own.result,fault('open_failed','unknown'));
});
test('连接前后取消区分效果；固定总等待不因零碎数据延长',async()=>{
  for(const sent of [false,true]){
    const h=harness(),own=h.start('about:blank',sent);
    h.controller.abort(Object.assign(Error('PRIVATE'),{code:'cancelled'}));
    await assert.rejects(own.result,fault('cancelled',sent?'unknown':'not_started'));
    assert.equal(own.socket.destroyed,true);
  }
  const h=harness(),own=h.start();own.socket.emit('data','{');
  assert.equal(h.timers[0].ms,5000);h.timers[0].fn();
  await assert.rejects(own.result,fault('timeout','unknown'));
  assert.equal(h.sockets.length,1);
});
test('并发与连接失败只返回一次错误，不排队、不重试',async()=>{
  const h=harness(),own=h.start();
  await assert.rejects(h.open('about:blank'),fault('open_busy','not_started'));
  own.socket.emit('error',Error('PRIVATE'));
  await assert.rejects(own.result,fault('native_bridge_failed','unknown'));
  assert.equal(h.sockets.length,1);
  const limited=harness();limited.scope.nativeId=128;
  await assert.rejects(limited.open('about:blank'),fault('native_call_limit','not_started'));
  assert.equal(limited.sockets.length,0);
});
