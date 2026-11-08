import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const source=await readFile(new URL('../../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const create=Function(source+'\nreturn createDesktopApi;')();
const identity=n=>({window_handle:String(n),process_id:n+10,thread_id:n+20,process_created_file_time:String(n+30)});
const a={...identity(11),expires_at_unix_ms:9999999999999};
const b={capture_id:1,context:{...identity(12),expires_at_unix_ms:9999999999998}};
const pointer={sampledAt:'call-time',coordinateSpace:'physical-screen',point:{x:-700,y:200},inputSource:{kind:'polled-pointer',injected:null,selfInjected:null},pointingWindow:identity(12),rootWindow:identity(12)};
const stopped={stopped:true,reason:'update_limit',acceptedUpdates:1,pollSamples:2,effectState:'unknown',dragCompleted:false};
const options=()=>({mode:'move',button:'left',maxDurationMs:20,maxUpdates:1,snap:null});
function harness(asynchronous,error){
 const calls=[];
 const invoke=(method,params)=>{
  calls.push({method,params:JSON.parse(JSON.stringify(params))});
  if(method==='target.context')return a;
  if(method==='pointer.root')return pointer;
  if(method==='target.handoff_pointing_root')return {requested:true,requestAccepted:true,foregroundConfirmed:true,target:b,reason:null,effectState:'unknown'};
  assert.equal(method,'window.drag');
  if(error)throw Object.assign(Error('private-host-error'),error);
  return stopped;
 };
 const {desktop}=create((method,params)=>asynchronous?Promise.resolve().then(()=>invoke(method,params)):invoke(method,params));
 return {desktop,calls,lease:async()=> (await desktop.handoffPointingRoot(pointer,{validForMs:5000,confirmTimeoutMs:50})).target};
}
for(const asynchronous of [false,true]){
 const label=asynchronous?'Node':'Script';
 test(label+'：省略或undefined保持原wire逐字且返回形状不变',async()=>{
  for(const input of [options(),{...options(),initialPointer:undefined}]){
   const h=harness(asynchronous),value=h.desktop.trackWindowDrag(input);
   assert.equal(Boolean(value&&typeof value.then==='function'),asynchronous);
   assert.deepEqual(await value,stopped);
   assert.equal(JSON.stringify(h.calls[0].params),'{"mode":"move","button":"left","max_duration_ms":20,"max_updates":1}');
  }
 });
 test(label+'：默认A和For路径仅添加i32坐标，复制后不随等待期间原对象变化',async()=>{
  for(const captured of [false,true]){
   const h=harness(asynchronous),target=captured?await h.lease():null;h.calls.length=0;
   const point={x:-2147483648,y:2147483647},input={...options(),initialPointer:point};
   const call=captured?h.desktop.trackWindowDragFor(target,input):h.desktop.trackWindowDrag(input);
   point.x=100;await call;
   assert.deepEqual(h.calls[0],{method:'window.drag',params:{mode:'move',button:'left',max_duration_ms:20,max_updates:1,initial_pointer:{x:-2147483648,y:2147483647},...(captured?{target:b}:{})}});
  }
 });
 test(label+'：无效坐标/形状/按钮元数据在bridge前拒绝，不做隐式转换',async()=>{
  let coerced=0;
  const object={valueOf(){coerced++;return 0;}};
  for(const point of [null,[],0,{}, {x:0},{x:0,y:0,left:'down'},{x:2147483648,y:0},{x:0,y:-2147483649},{x:0.5,y:0},{x:NaN,y:0},{x:Infinity,y:0},{x:'0',y:0},{x:object,y:0}]){
   const h=harness(asynchronous);
   await assert.rejects(async()=>h.desktop.trackWindowDrag({...options(),initialPointer:point}),e=>e.code==='invalid_input'&&e.method==='window.drag'&&e.effectState==='not_started');
   assert.equal(h.calls.length,0);
  }
  const h=harness(asynchronous),point={y:0};let reads=0;Object.defineProperty(point,'x',{enumerable:true,get(){reads++;return 0;}});
  await assert.rejects(async()=>h.desktop.trackWindowDrag({...options(),initialPointer:point}),{code:'invalid_input'});
  assert.equal(reads,0);assert.equal(coerced,0);
 });
 test(label+'：提供锚点不放宽持续/提交预算',async()=>{
  for(const patch of [{maxDurationMs:19},{maxDurationMs:2001},{maxUpdates:0},{maxUpdates:65}]){
   const h=harness(asynchronous);
   await assert.rejects(async()=>h.desktop.trackWindowDrag({...options(),...patch,initialPointer:{x:0,y:0}}),{code:'invalid_input'});
   assert.equal(h.calls.length,0);
  }
 });
 test(label+'：For租约/取消/期限错误保持原样，unknown不重试或回退A',async()=>{
  for(const code of ['target_expired','target_changed','cancelled','timeout']){
   for(const effectState of ['not_started','unknown']){
    const h=harness(asynchronous,{code,effectState}),target=await h.lease();h.calls.length=0;
    await assert.rejects(async()=>h.desktop.trackWindowDragFor(target,{...options(),initialPointer:{x:-700,y:200}}),e=>e.code===code&&e.effectState===effectState&&e.method==='window.drag'&&!e.message.includes('private'));
    assert.equal(h.calls.length,1);assert.deepEqual(h.calls[0].params.target,b);
   }
  }
 });
 test(label+'：显式For同样在传输前拒绝无效起点，不消费已有租约',async()=>{
  for(const point of [null,{x:0,y:0,epoch:1},{x:0,y:2147483648}]){
   const h=harness(asynchronous),target=await h.lease();h.calls.length=0;
   await assert.rejects(async()=>h.desktop.trackWindowDragFor(target,{...options(),initialPointer:point}),e=>e.code==='invalid_input'&&e.method==='window.drag'&&e.effectState==='not_started');
   assert.equal(h.calls.length,0);
   await h.desktop.trackWindowDragFor(target,{...options(),initialPointer:{x:-700,y:200}});
   assert.equal(h.calls.length,1);assert.deepEqual(h.calls[0].params.target,b);
  }
 });
}
