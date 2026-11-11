import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
import {runSizeStep} from '../../examples/actions/window-size-step/flow.mjs';
const sdk=readFileSync(new URL('../../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const factory=new vm.Script(`(()=>{${sdk};return createDesktopApi;})()`).runInThisContext();
const identity=b=>({window_handle:b?'202':'101',process_id:b?20:10,thread_id:b?21:11,process_created_file_time:b?'18446744073709551615':'134000000000000001'});
const original=b=>({...identity(b),expires_at_unix_ms:9999999999999});
const rect=(width=801,height=601)=>({left:-800,top:100,width,height});
function recording(captured,maximized){
 const calls=[],callFrames=[];let restored=false;const token={capture_id:7,context:original(true)};
 const desktop=factory((method,params)=>{
  calls.push({method,params:structuredClone(params)});
  callFrames.push(new Error().stack.split('\n').filter(line=>line.includes('/window-size-step/flow.mjs:')&&/\bat call\b/.test(line)).length);
  if(method==='target.context')return original(false);
  if(method==='pointer.root')return {sampledAt:'call-time',coordinateSpace:'physical-screen',point:{x:1,y:1},inputSource:{kind:'polled-pointer',injected:null,selfInjected:null},pointingWindow:identity(captured),rootWindow:identity(captured)};
  if(method==='target.handoff_pointing_root')return {requested:true,requestAccepted:true,foregroundConfirmed:true,target:token,reason:null,effectState:'unknown'};
  if(method==='window.geometry'){
   const current=restored?rect(400,300):rect();
   return {coordinateSpace:'physical-screen',className:'TestWindow',outerRect:current,visibleFrameRect:current,monitorRect:rect(1920,1080),workArea:rect(1920,1040),primaryScreen:{width:2560,height:1440},windowDpi:96,maximized:maximized&&!restored};
  }
  if(method==='window.action'){assert.equal(params.operation,'restore');restored=true;return {submitted:true};}
  if(method==='window.set_rect')return {submitted:true,effectState:'unknown'};
  assert.fail(method);
 }).desktop;
 const ctx={desktop,input:{trigger:'AdvancedMouseAction',step:50,targetPolicy:captured?'handoff-pointing-root':'original-only',...(captured?{handoff:{validForMs:5000,confirmTimeoutMs:1000}}:{})},signal:{aborted:false}};
 return {ctx,calls,callFrames,token};
}
for(const captured of [false,true])for(const maximized of [false,true])for(const operation of ['increase','decrease'])test(`同步flow逐步退栈 ${captured?'B':'A'} ${maximized?'restore':'normal'} ${operation}`,()=>{
 const h=recording(captured,maximized),limit=Error.stackTraceLimit;let result;
 // 只扩大V8诊断记录数量，不改变任何宿主栈、内存或执行预算。
 try{Error.stackTraceLimit=200;result=runSizeStep(h.ctx,false,operation);}finally{Error.stackTraceLimit=limit;}
 assert.equal(Boolean(result?.then),false);assert.equal(result.status,'size_submitted');assert.equal(result.windowChangeConfirmed,false);
 assert.deepEqual(result.plannedRect,rect(operation==='increase'?851:751,operation==='increase'?651:551));
 assert.equal(h.calls.filter(c=>c.method==='window.set_rect').length,1);
 assert.equal(h.calls.filter(c=>c.method==='window.action').length,maximized?1:0);
 assert.equal(h.calls.filter(c=>c.method==='target.handoff_pointing_root').length,captured?1:0);
 for(const call of h.calls.filter(c=>c.method.startsWith('window.')))assert.deepEqual(call.params.target,captured?h.token:undefined);
 assert(h.callFrames.some(count=>count===1));
 assert(h.callFrames.every(count=>count<=1),'同步flow调用帧应在推进前退出');
});
