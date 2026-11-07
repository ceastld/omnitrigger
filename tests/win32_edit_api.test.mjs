import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs';
import vm from 'node:vm';
import {EventEmitter} from 'node:events';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const source = rel => fs.readFileSync(path.join(root, rel), 'utf8');
const desktopSource = source('crates/omnitrigger-actions/runtime/desktop-api.js');
const host = source('crates/omnitrigger-actions/runtime/host.mjs');
const sdk = source('crates/omnitrigger-actions/runtime/actions-api.js')+'\n'+source('tools/js-engine-probe/src/sdk.js');
const asyncApiLine = host.split('\n').find(line => line.startsWith('const asyncApi ='));
function core(invoke, asynchronous=false) {
  const context = vm.createContext({invoke});
  vm.runInContext(desktopSource+'\n'+asyncApiLine+'\nglobalThis.api='+(asynchronous?'asyncApi(createDesktopApi(invoke).desktop)':'createDesktopApi(invoke).desktop')+';', context);
  return context.api;
}
const json = value => JSON.parse(JSON.stringify(value));
const failure = (code, effectState='unknown') => Object.assign(new Error('private message'),{code,effectState,method:'wrong.method',requiredPermissions:['secret']});
async function reject(promise, code, effect='unknown') {
  await assert.rejects(promise, err => {
    assert.equal(err.code,code);assert.equal(err.method,'win32_edit.replace_selection');
    assert.equal(err.effectState,effect);assert.equal(err.requiredPermissions,undefined);
    assert(!err.message.includes('private'));return true;
  });
}

function syncRuntime({bridge=true, reply=()=>({messageProcessed:true,effectState:'unknown'})}={}) {
  const calls=[];
  const context=vm.createContext({
    ctx:bridge?{__nativeContext:{connection:{port:9,token:'a'.repeat(64)}}}:{},
    __hostCall:serialized=>{
      const request=JSON.parse(serialized);calls.push(request);
      try { return JSON.stringify({ok:true,value:request.kind==='native'?reply(request):null}); }
      catch (error) { return JSON.stringify({ok:false,error:{code:error.code,message:'redacted',effect_state:error.effectState,method:error.method}}); }
    },
    createProcessCaptureApi:()=>({capture(){}}),createBrowserApi:()=>({}),createClipboardHistoryApi:()=>({}),
  });
  vm.runInContext(desktopSource+'\n'+sdk, context);
  return {ctx:context.ctx,calls};
}

function nodeRuntime({bridge=true, mode='ok',value={messageProcessed:true,effectState:'unknown'},nativeError}={}) {
  const calls=[],timers=new Map(),sockets=[];
  class Socket extends EventEmitter {
    setEncoding(){} setTimeout(_ms,callback){this.timeoutMs=_ms;this.timeout=callback;}
    destroy(){this.destroyed=true;}
    write(frame){
      const request=JSON.parse(frame);calls.push(request);
      if(mode==='ok') queueMicrotask(()=>this.emit('data',JSON.stringify(nativeError?{id:request.id,ok:false,error:nativeError}:{id:request.id,ok:true,value})+'\n'));
    }
  }
  const controller=new AbortController();
  const context=vm.createContext({
    request:bridge?{native:{port:9,token:'a'.repeat(64)}}:{},controller,Buffer,
    fault:(code,message)=>Object.assign(new Error(message??code),{code}),
    checkCancelled:()=>{if(controller.signal.aborted)throw controller.signal.reason;},
    createConnection:()=>{const socket=new Socket();sockets.push(socket);queueMicrotask(()=>socket.emit('connect'));return socket;},
    setTimeout:callback=>{const id={};timers.set(id,callback);return id;},clearTimeout:id=>timers.delete(id),
  });
  const setStart=host.indexOf('const nativeMethods =');
  const methods=host.slice(setStart,host.indexOf(']);',setStart)+3);
  const fnStart=host.indexOf('function nativeCall(');
  const native=host.slice(fnStart,host.indexOf('\n}',fnStart)+2);
  vm.runInContext('let nativeId=0,nativePending,nativePendingMethod;const nativeSockets=new Set();\n'+methods+'\n'+native+'\n'+desktopSource+'\n'+asyncApiLine+'\nglobalThis.api=asyncApi(createDesktopApi(nativeCall).desktop);globalThis.exhaust=()=>{nativeId=128;};',context);
  return {api:context.api,calls,timers,sockets,controller,exhaust:context.exhaust};
}

const options={adapter:'native-edit'},ok={messageProcessed:true,effectState:'unknown'};
await test('实际Node共享SDK与nativeCall发送一次窄方法',async()=>{
  const e=nodeRuntime();assert.deepEqual(json(await e.api.win32EditReplaceSelection('A\r\nB😀',options)),ok);
  assert.equal(e.calls.length,1);assert.equal(e.calls[0].method,'win32_edit.replace_selection');
  assert.deepEqual(e.calls[0].params,{text:'A\r\nB😀',adapter:'native-edit'});assert.equal(e.timers.size,0);
});
await test('完整同步SDK使用native kind且直接返回冻结窄回执',async()=>{
  const e=syncRuntime();const value=e.ctx.desktop.win32EditReplaceSelection('x',{adapter:'winforms-edit'});
  assert.equal(typeof value.then,'undefined');assert(Object.isFrozen(value));assert.deepEqual(json(value),ok);
  assert.equal(e.calls.length,1);assert.equal(e.calls[0].kind,'native');assert.equal(e.calls[0].method,'win32_edit.replace_selection');
});
await test('必填适配器与严格参数形状在零桥接时拒绝',async()=>{
  let calls=0;const api=core(()=>{calls++;return ok;},true);
  for(const args of [[],['x'],['x',null],['x',{}],['x',{adapter:'auto'}],['x',{adapter:'Edit'}],['x',{adapter:'native-edit',hwnd:10}],
    ['x',{adapter:'native-edit',[Symbol('x')]:true}],['x',Object.defineProperty({},'adapter',{get(){throw Error('private');}})],
    [5,options],['x',options,{}]]) await reject(api.win32EditReplaceSelection(...args),'invalid_input','not_started');
  assert.equal(calls,0);
});
await test('UTF16上限NUL孤立代理项与空串语义',async()=>{
  const calls=[];const api=core((method,params)=>{calls.push({method,params});return ok;},true);
  for(const text of ['','😀'.repeat(4096),'x'.repeat(8192)])assert.deepEqual(json(await api.win32EditReplaceSelection(text,options)),ok);
  for(const text of ['x'.repeat(8193),'😀'.repeat(4097),'x\0y','\ud800','\udfff','\ud800x'])await reject(api.win32EditReplaceSelection(text,options),'invalid_input','not_started');
  assert.equal(calls.length,3);assert.equal(calls[0].params.text,'');
});
await test('适配器在提交时复制不受调用方后续修改影响',async()=>{
  const e=nodeRuntime();const mutable={adapter:'native-edit'};const p=e.api.win32EditReplaceSelection('x',mutable);mutable.adapter='winforms-edit';
  await p;assert.equal(e.calls[0].params.adapter,'native-edit');
});
await test('窄回执拒绝原文身份额外字段和错误效果声明',async()=>{
  for(const value of [null,{},true,{messageProcessed:false,effectState:'unknown'},{messageProcessed:true,effectState:'not_started'},
    {messageProcessed:true,effectState:'unknown',text:'private'},{messageProcessed:true,effectState:'unknown',hwnd:1}]){
    const api=core(()=>value,true);await reject(api.win32EditReplaceSelection('x',options),'invalid_response');
  }
  let read=0;const value=Object.defineProperty({effectState:'unknown'},'messageProcessed',{enumerable:true,get(){read++;return true;}});
  await reject(core(()=>value,true).win32EditReplaceSelection('x',options),'invalid_response');assert.equal(read,0);
});
await test('错误保留已知代码和效果但移除正文权限及伪造方法',async()=>{
  for(const [code,effect] of [['target_changed','not_started'],['edit_unsupported','not_started'],['cancelled','not_started'],['timeout','unknown'],['permission_denied','unknown']]){
    const api=core(()=>{throw failure(code,effect);},true);await reject(api.win32EditReplaceSelection('x',options),code,effect);
  }
  await reject(core(()=>{throw failure('private_text_as_code');},true).win32EditReplaceSelection('x',options),'native_bridge_failed');
});
await test('异常getter不求值不回显私人正文',async()=>{
  let reads=0;const error=Object.defineProperties(new Error('private'),{code:{get(){reads++;return 'timeout';}},effectState:{get(){reads++;return 'not_started';}}});
  await reject(core(()=>{throw error;},true).win32EditReplaceSelection('x',options),'native_bridge_failed');assert.equal(reads,0);
});
await test('两种运行时无Native桥接均明确拒绝不转其他能力',async()=>{
  const n=nodeRuntime({bridge:false});await reject(n.api.win32EditReplaceSelection('x',options),'native_bridge_unavailable','not_started');assert.equal(n.calls.length,0);
  const s=syncRuntime({bridge:false});assert.throws(()=>s.ctx.desktop.win32EditReplaceSelection('x',options),e=>e.code==='native_bridge_unavailable'&&e.effectState==='not_started');assert.equal(s.calls.length,0);
});
await test('Node沿用128预算与单pending门闩',async()=>{
  const limit=nodeRuntime();limit.exhaust();await reject(limit.api.win32EditReplaceSelection('x',options),'desktop_call_limit','not_started');assert.equal(limit.calls.length,0);
  const e=nodeRuntime({mode:'hold'});const first=e.api.win32EditReplaceSelection('x',options);await new Promise(setImmediate);
  await reject(e.api.win32EditReplaceSelection('x',options),'desktop_busy','not_started');
  e.sockets[0].emit('data',JSON.stringify({id:1,ok:true,value:ok})+'\n');await first;assert.equal(e.calls.length,1);
});
await test('同步共用32宿主调用预算不为新方法重置',async()=>{
  const e=syncRuntime();for(let i=0;i<32;i++)e.ctx.delay(0);
  assert.throws(()=>e.ctx.desktop.win32EditReplaceSelection('x',options),x=>x.code==='desktop_call_limit'&&x.effectState==='not_started');
  assert.equal(e.calls.length,32);assert(e.calls.every(x=>x.kind==='delay'));
});
await test('Node五秒等待不增长且提交后超时unknown无重试',async()=>{
  const e=nodeRuntime({mode:'hold'});const p=e.api.win32EditReplaceSelection('x',options);await new Promise(setImmediate);
  assert.equal(e.sockets.length,1);assert.equal(e.sockets[0].timeoutMs,5000);e.sockets[0].timeout();await reject(p,'timeout');assert.equal(e.calls.length,1);assert.equal(e.timers.size,0);
});
await test('Node发送后取消与发送前取消准确标效果',async()=>{
  const e=nodeRuntime({mode:'hold'});const p=e.api.win32EditReplaceSelection('x',options);await new Promise(setImmediate);
  e.controller.abort(Object.assign(new Error('private'),{code:'cancelled'}));await reject(p,'cancelled');assert.equal(e.calls.length,1);
  const early=nodeRuntime();early.controller.abort(Object.assign(new Error('private'),{code:'cancelled'}));
  await reject(early.api.win32EditReplaceSelection('x',options),'cancelled','not_started');assert.equal(early.calls.length,0);
});
await test('实际Node帧错误保留结构化效果并清除权限原文',async()=>{
  const e=nodeRuntime({nativeError:{code:'edit_unsupported',message:'private',method:'win32_edit.replace_selection',effect_state:'not_started',required_permissions:['private']}});
  await reject(e.api.win32EditReplaceSelection('x',options),'edit_unsupported','not_started');assert.equal(e.calls.length,1);
});
await test('同步响应缺少效果时新方法不能声称not_started',async()=>{
  const e=syncRuntime({reply:()=>{throw Object.assign(new Error('private'),{code:'timeout'});}});
  assert.throws(()=>e.ctx.desktop.win32EditReplaceSelection('x',options),x=>x.code==='timeout'&&x.effectState==='unknown'&&!x.message.includes('private'));
});


await test('Node错误方法回执拒绝且保守标为unknown，不重试', async () => {
  const e=nodeRuntime({nativeError:{code:'edit_unsupported',message:'private original',method:'wrong.method',effect_state:'not_started',required_permissions:['private']}});
  await reject(e.api.win32EditReplaceSelection('x',options),'invalid_response','unknown');
  assert.equal(e.calls.length,1);
});
