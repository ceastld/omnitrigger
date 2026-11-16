import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const text=await readFile(new URL('../../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const create=Function(text+'\nreturn createDesktopApi;')();
const rect={left:-800,top:100,width:800,height:600};
const geometry={coordinateSpace:'physical-screen',className:'TestWindow',outerRect:rect,visibleFrameRect:rect,monitorRect:rect,workArea:rect,primaryScreen:{width:2560,height:1440},windowDpi:144,maximized:false};
const state={coordinateSpace:'physical-screen',x:-700,y:200,buttons:{left:'down',right:'not_down_or_unavailable',middle:'not_down_or_unavailable'},inputSource:{kind:'polled-key-state',injected:null,selfInjected:null}};
const stopped={stopped:true,reason:'update_limit',acceptedUpdates:1,pollSamples:2,effectState:'unknown',dragCompleted:false};
function harness(asynchronous,override={}){
 const calls=[];
 const invoke=(method,params)=>{calls.push({method,params});return method in override?override[method]:method==='window.geometry'?geometry:method==='pointer.state'?state:method==='window.set_rect'?{submitted:true,effectState:'unknown'}:method==='window.drag'?stopped:{submitted:true};};
 const {desktop}=create((method,params)=>asynchronous?Promise.resolve().then(()=>invoke(method,params)):invoke(method,params));
 return {desktop,calls};
}
for(const asynchronous of [false,true]){
 const label=asynchronous?'Node':'Script';
 test(label+'：几何、矩形、指针与有界调整使用精确wire和回执',async()=>{
  const h=harness(asynchronous),call=h.desktop.getWindowGeometry();assert.equal(Boolean(call?.then),asynchronous);
  assert.deepEqual(await call,geometry);assert.deepEqual(await h.desktop.getPointerState(),state);
  assert.deepEqual(await h.desktop.setWindowRect(rect,{frame:'visible'}),{submitted:true,effectState:'unknown'});
  assert.deepEqual(await h.desktop.trackWindowDrag({mode:'move',button:'left',maxDurationMs:20,maxUpdates:1,snap:null}),stopped);
  assert.deepEqual(h.calls.map(x=>x.method),['window.geometry','pointer.state','window.set_rect','window.drag']);
  assert.deepEqual(h.calls[2].params,{rect,frame:'visible'});
  assert.deepEqual(h.calls[3].params,{mode:'move',button:'left',max_duration_ms:20,max_updates:1});
 });
 test(label+'：尺寸、坐标、预算及未知字段在bridge前拒绝',async()=>{
  const h=harness(asynchronous);
  for(const bad of [{...rect,width:0},{...rect,width:65536},{...rect,left:2147483647}]){
   await assert.rejects(async()=>h.desktop.setWindowRect(bad,{frame:'visible'}),e=>e.code==='invalid_input'&&e.effectState==='not_started');
  }
  for(const patch of [{maxDurationMs:19},{maxDurationMs:2001},{maxUpdates:0},{initialPointer:{x:0,y:0,left:'down'}}]){
   await assert.rejects(async()=>h.desktop.trackWindowDrag({mode:'move',button:'left',maxDurationMs:20,maxUpdates:1,...patch}),{code:'invalid_input'});
  }
  assert.equal(h.calls.length,0);
 });
 test(label+'：不完整回执以读not_started/写unknown失败，不回退或重试',async()=>{
  const h=harness(asynchronous,{'window.geometry':{...geometry,primaryScreen:undefined},'window.set_rect':{submitted:true},'window.drag':{...stopped,dragCompleted:true},'pointer.state':{...state,buttons:{left:'up'}}});
  for(const [method,invoke,effect]of [['window.geometry',()=>h.desktop.getWindowGeometry(),'not_started'],['pointer.state',()=>h.desktop.getPointerState(),'not_started'],['window.set_rect',()=>h.desktop.setWindowRect(rect,{frame:'outer'}),'unknown'],['window.drag',()=>h.desktop.trackWindowDrag({mode:'resize',button:'left',maxDurationMs:20,maxUpdates:1}),'unknown']]){
   await assert.rejects(async()=>invoke(),e=>e.code==='invalid_response'&&e.method===method&&e.effectState===effect);
  }
  assert.equal(h.calls.length,4);
 });
}
test('Script完整bootstrap的false Native标记在传输前拒绝六个window wire',async()=>{
 const sdk=await readFile(new URL('../../tools/js-engine-probe/src/sdk.js',import.meta.url),'utf8');
 const body=sdk.slice(sdk.indexOf('function desktopCall('),sdk.indexOf('ctx.state ='));
 const invoke=Function('method','let calls=0,nativeId=0;const actionsScope=true,nativeContext={connection:{}},nativeContextAvailable=false;function call(){throw Error("不得进入传输");}function encode(){throw Error("不得序列化");}'+body+';return desktopCall(method,{});');
 for(const method of ['window.geometry','window.set_rect','pointer.state','pointer.root','target.handoff_pointing_root','window.drag'])assert.throws(()=>invoke(method),e=>e.code==='native_bridge_unavailable'&&e.effectState==='not_started');
});
test('公开SDK显式起点只接受坐标，不接受按钮或epoch证明',async()=>{
 const h=harness(false),base={mode:'move',button:'left',maxDurationMs:20,maxUpdates:1};
 await h.desktop.trackWindowDrag({...base,initialPointer:{x:-700,y:200}});
 assert.deepEqual(h.calls[0].params.initial_pointer,{x:-700,y:200});
 for(const extra of [{left:'down'},{epoch:1},{injected:false}]){
  await assert.rejects(async()=>h.desktop.trackWindowDrag({...base,initialPointer:{x:0,y:0,...extra}}),{code:'invalid_input'});
 }
 assert.equal(h.calls.length,1);
});
test('Node传输的false Native标记拒绝新wire及显式window.action，保持默认旧分支',async()=>{
 const source=await readFile(new URL('../../crates/omnitrigger-actions/runtime/host.mjs',import.meta.url),'utf8');
 const start=source.indexOf('function nativeCall('),end=source.indexOf('  if (nativePending)',start);
 const prefix=source.slice(start,end)+'\n}\n';
 const methods=['window.geometry','window.set_rect','pointer.state','pointer.root','target.handoff_pointing_root','window.drag'];
 const invoke=Function('method','params','const request={native:{},native_context_available:false};const controller={signal:{aborted:false}};const nativeMethods=new Set('+JSON.stringify([...methods,'window.action'])+');function checkCancelled(){}function fault(code,message){return Object.assign(Error(message),{code});}'+prefix+'return nativeCall(method,params);');
 for(const method of methods)await assert.rejects(()=>invoke(method,{}),e=>e.code==='native_bridge_unavailable'&&e.effectState==='not_started');
 await assert.rejects(()=>invoke('window.action',{target:{}}),{code:'native_bridge_unavailable'});
 await assert.rejects(()=>invoke('window.action',{}),{code:'desktop_unavailable'});
});
for(const asynchronous of [false,true]){
 test((asynchronous?'Node':'Script')+'实际original fixture覆盖默认A新接口',async()=>{
  const a={window_handle:'11',process_id:21,thread_id:31,process_created_file_time:'41',expires_at_unix_ms:9999999999999},calls=[];
  const invoke=(method,params)=>{
   calls.push({method,params});assert(!Object.hasOwn(params,'target'));
   if(method==='target.context')return a;
   if(method==='pointer.state')return state;
   if(method==='window.geometry')return geometry;
   if(method==='window.set_rect')return {submitted:true,effectState:'unknown'};
   if(method==='window.action')return {submitted:true};
   if(method==='window.drag')return stopped;
   assert.fail(method);
  };
  const {desktop}=create((method,params)=>asynchronous?Promise.resolve().then(()=>invoke(method,params)):invoke(method,params));
  const module=await import('../../crates/omnitrigger-actions/tests/fixtures/captured-window-runtimes/'+(asynchronous?'node':'script')+'.mjs');
  const result=await (asynchronous?module.default:module.main)({input:{originalOnly:true},desktop});
  assert.equal(result.status,'original_completed');assert.deepEqual(result.before,result.after);
  assert.deepEqual(calls.map(x=>x.method),['target.context','pointer.state','window.geometry','window.set_rect','window.action','window.drag','target.context']);
 });
}
