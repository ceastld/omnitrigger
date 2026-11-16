import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const source=await readFile(new URL('../../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const create=Function(source+'\nreturn createDesktopApi;')();
const identity=n=>({window_handle:String(n),process_id:n+10,thread_id:n+20,process_created_file_time:String(n+30)});
const a={...identity(11),expires_at_unix_ms:9999999999999};
const b={capture_id:1,context:{...identity(12),expires_at_unix_ms:9999999999998}};
const expected={sampledAt:'call-time',coordinateSpace:'physical-screen',point:{x:-700,y:200},inputSource:{kind:'polled-pointer',injected:null,selfInjected:null},pointingWindow:identity(12),rootWindow:identity(12)};
function harness(asynchronous,options={}){
 const calls=[];
 const invoke=(method,params)=>{
  calls.push({method,params:JSON.parse(JSON.stringify(params))});
  if(method==='target.context')return a;
  if(method==='pointer.root')return expected;
  if(method==='target.handoff_pointing_root')return {requested:true,requestAccepted:true,foregroundConfirmed:true,target:b,reason:null,effectState:'unknown'};
  assert.equal(method,'window.action');assert.deepEqual(params,{operation:'toggle_maximize',target:b});
  if(options.error)throw Object.assign(Error('private-host-message'),options.error);
  return Object.hasOwn(options,'reply')?options.reply:{submitted:true};
 };
 const {desktop}=create((method,params)=>asynchronous?Promise.resolve().then(()=>invoke(method,params)):invoke(method,params));
 const handoff=()=>desktop.handoffPointingRoot(expected,{validForMs:5000,confirmTimeoutMs:50});
 return {desktop,calls,handoff};
}
for(const asynchronous of [false,true]){
 const label=asynchronous?'Node':'Script';
 test(label+'：最窄For方法沿同一window.action传B，默认A不变',async()=>{
  const h=harness(asynchronous);const before=await h.desktop.targetContext(),receipt=await h.handoff();
  const result=h.desktop.toggleMaximizeWindowFor(receipt.target);
  assert.equal(Boolean(result&&typeof result.then==='function'),asynchronous);
  assert.deepEqual(await result,{submitted:true});assert.deepEqual(await h.desktop.targetContext(),before);
  assert.deepEqual(h.calls.map(x=>x.method),['target.context','target.handoff_pointing_root','window.action','target.context']);
  assert.deepEqual(h.calls[2].params,{operation:'toggle_maximize',target:b});
 });
 test(label+'：参数数量、裸句柄、无效DTO在bridge前拒绝',async()=>{
  const h=harness(asynchronous);const target=(await h.handoff()).target;
  h.calls.length=0;
  for(const args of [[],[target,{}],[11],[{capture_id:0,context:b.context}],[{...b,extra:1}],[{capture_id:1,context:{...b.context,process_id:0}}]]){
   await assert.rejects(async()=>h.desktop.toggleMaximizeWindowFor(...args),e=>e.code==='invalid_input'&&e.method==='window.action'&&e.effectState==='not_started');
  }
  assert.equal(h.calls.length,0);
 });
 test(label+'：DTO副本保留原合同，租约归属由宿主核验',async()=>{
  const h=harness(asynchronous),target=(await h.handoff()).target;h.calls.length=0;
  assert.deepEqual(await h.desktop.toggleMaximizeWindowFor({...target,context:{...target.context}}),{submitted:true});
  assert.equal(h.calls.length,1);
  const rejected=harness(asynchronous,{error:{code:'target_changed',effectState:'not_started'}});
  await assert.rejects(async()=>rejected.desktop.toggleMaximizeWindowFor({...target,context:{...target.context}}),e=>e.code==='target_changed'&&e.effectState==='not_started');
  assert.equal(rejected.calls.length,1);
 });
 test(label+'：B过期/错身份/取消/期限等守卫错误原样结构化，不重试或回退A',async()=>{
  for(const code of ['target_expired','target_changed','cancelled','timeout','target_unavailable']){
   const h=harness(asynchronous,{error:{code,method:'window.action',effectState:'not_started'}});
   const target=(await h.handoff()).target;h.calls.length=0;
   await assert.rejects(async()=>h.desktop.toggleMaximizeWindowFor(target),e=>e.code===code&&e.method==='window.action'&&e.effectState==='not_started'&&!e.message.includes('private'));
   assert.equal(h.calls.length,1);
  }
 });
 test(label+'：unknown和坏回执不重试、不改A、不吞成已提交',async()=>{
  const unknown=harness(asynchronous,{error:{code:'timeout',effectState:'unknown'}}),target=(await unknown.handoff()).target;unknown.calls.length=0;
  await assert.rejects(async()=>unknown.desktop.toggleMaximizeWindowFor(target),e=>e.code==='timeout'&&e.effectState==='unknown');
  assert.equal(unknown.calls.length,1);
  for(const reply of [null,false,[],{}, {submitted:false},{submitted:true,extra:1}]){
   const h=harness(asynchronous,{reply}),target=(await h.handoff()).target;h.calls.length=0;
   await assert.rejects(async()=>h.desktop.toggleMaximizeWindowFor(target),e=>e.code==='invalid_response'&&e.method==='window.action'&&e.effectState==='unknown');
   assert.equal(h.calls.length,1);
  }
 });
 test(label+'：原四个For方法仍保留',()=>{
  const h=harness(asynchronous);
  for(const name of ['getWindowGeometryFor','setWindowRectFor','restoreWindowFor','trackWindowDragFor'])assert.equal(typeof h.desktop[name],'function');
 });
 test(label+'：录制Host所用实际fixture在纯桥上走新分支',async()=>{
  const h=harness(asynchronous);
  const module=await import('../../crates/omnitrigger-actions/tests/fixtures/captured-window-runtimes/'+(asynchronous?'node':'script')+'.mjs');
  const output=await (asynchronous?module.default:module.main)({input:{toggleOnly:true},desktop:h.desktop});
  assert.deepEqual(output,{status:'toggle_completed',before:a,after:a,target:b,shape:asynchronous,toggle:{submitted:true}});
  assert.deepEqual(h.calls.map(x=>x.method),['target.context','pointer.root','target.handoff_pointing_root','window.action','target.context']);
 });
}
