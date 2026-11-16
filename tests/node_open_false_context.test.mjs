// 只评估正式客户端函数；socket、计时器与原生回执全为内存替身，不打开任何目标。
import assert from 'node:assert/strict';
import {EventEmitter} from 'node:events';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
const host=readFileSync(new URL('../crates/omnitrigger-actions/runtime/host.mjs', import.meta.url),'utf8');
const body=host.slice(host.indexOf('function nativeCall('),host.indexOf('const desktopApis ='));
const plain=value=>JSON.parse(JSON.stringify(value));
function harness(fileParent=false){
  const sockets=[],timers=[];
  class Socket extends EventEmitter{
    setEncoding(value){this.encoding=value;}
    setTimeout(ms,handler){this.timeout=ms;this.expire=handler;}
    write(text){this.request=JSON.parse(text);}
    destroy(){this.destroyed=true;}
  }
  const controller=new AbortController();
  const scope={Buffer,TextDecoder,Promise,Set,controller,nativeMethods:new Set(['system.open','system.open_file_in_parent']),
    nativeSockets:new Set(),nativeId:0,nativePending:undefined,nativePendingMethod:undefined,
    request:{native:{port:1,token:'synthetic'}},
    fault:(code,message)=>Object.assign(Error(message),{code}),
    checkCancelled:()=>{if(controller.signal.aborted)throw controller.signal.reason;},
    createConnection:()=>{const socket=new Socket();sockets.push(socket);return socket;},
    setTimeout:(fn,ms)=>{const timer={fn,ms};timers.push(timer);return timer;},
    clearTimeout:timer=>{if(timer)timer.cleared=true;},
  };
  const open=runInNewContext(body+(fileParent?'\nopenFileInParent;':'\nopenTarget;'),scope);
  function start(target=fileParent?'C:/Owned/a.pdf':'https://example.invalid/',connect=true){
    const result=open(target);result.catch(()=>{});
    const socket=sockets.at(-1);
    if(connect)socket.emit('connect');
    return{result,socket};
  }
  const send=(socket,value)=>socket.emit('data',JSON.stringify({id:socket.request.id,...value})+'\n');
  return{open,start,send,scope,controller,sockets,timers};
}
function fault(code,effect,method='system.open'){return error=>{
  assert.equal(error.code,code);assert.equal(error.effectState,effect);
  assert.equal(error.method,method);assert(!error.message.includes('PRIVATE'));
  assert.equal(error.requiredPermissions,undefined);return true;
};}
for(const parent of [false,true]) {
  const method=parent?'system.open_file_in_parent':'system.open';
  const target=parent?'C:/OwnedMissing/never.txt':'https://synthetic.invalid/?q=%E4%B8%AD';
  test(`false/${method}成功只提交一次，保留原参数和false`,async()=>{
    const h=harness(parent);h.scope.request.native_context_available=false;
    const own=h.start(target);
    h.send(own.socket,{ok:true,value:{submitted:true}});
    assert.deepEqual(plain(await own.result),{submitted:true});
    assert.equal(h.sockets.length,1);assert.equal(own.socket.request.method,method);
    assert.deepEqual(own.socket.request.params,{target});
    assert.equal(h.scope.request.native_context_available,false);
  });
  for(const [code,effect] of [['open_unavailable','not_started'],['open_failed','unknown']])
  test(`false/${method}/${effect}保持Host错误，不重试`,async()=>{
    const h=harness(parent);h.scope.request.native_context_available=false;
    const own=h.start(target);
    h.send(own.socket,{ok:false,error:{code,effect_state:effect,method,message:'PRIVATE synthetic'}});
    await assert.rejects(own.result,fault(code,effect,method));
    assert.equal(h.sockets.length,1);assert.equal(h.scope.request.native_context_available,false);
  });
  test(`false/${method}坏回执unknown，无桥或构造错误not_started`,async()=>{
    const h=harness(parent);h.scope.request.native_context_available=false;
    const own=h.start(target);h.send(own.socket,{ok:true,value:{submitted:true,extra:true}});
    await assert.rejects(own.result,fault('invalid_response','unknown',method));
    assert.equal(h.sockets.length,1);
    for(const construction of [false,true]) {
      const missing=harness(parent);missing.scope.request.native_context_available=false;
      delete missing.scope.request.native;
      if(construction)missing.scope.request.native_error={code:'native_bridge_unavailable',message:'PRIVATE synthetic'};
      await assert.rejects(missing.open(target),fault('native_bridge_unavailable','not_started',method));
      assert.equal(missing.sockets.length,0);assert.equal(missing.scope.request.native_context_available,false);
    }
  });
}
test('false仍拒绝窗口与剪贴板，不创建socket',async()=>{
  for(const method of ['window.geometry','clipboard.read','clipboard.write']) {
    const h=harness();h.scope.request.native_context_available=false;h.scope.nativeMethods.add(method);
    const call=runInNewContext(body+'\nnativeCall;',h.scope);
    await assert.rejects(call(method,{}),e=>e.effectState==='not_started'&&e.method===method);
    assert.equal(h.sockets.length,0);assert.equal(h.scope.request.native_context_available,false);
  }
});
