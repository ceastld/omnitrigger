import test from 'node:test';
import assert from 'node:assert/strict';
import main from './main.mjs';
const id=n=>({window_handle:String(n),process_id:n+10,thread_id:n+20,process_created_file_time:String(n+30)});
const a=id(11),b=id(12),token={capture_id:1,context:{...b,expires_at_unix_ms:9999999999999}};
function harness(input={mode:'move',handoff:false,restore:false},pointed=false){
 const calls=[],signal={aborted:false};let maximized=false;
 const call=(method,value)=>{calls.push(method);return value;};
 const desktop={
  targetContext:async()=>call('original',{...a,expires_at_unix_ms:9999999999999}),
  getPointingRoot:async()=>call('pointing',{rootWindow:pointed?b:a}),
  handoffPointingRoot:async()=>call('handoff',{requestAccepted:true,foregroundConfirmed:true,target:token}),
  getWindowGeometry:async()=>call('geometry',{maximized}),
  getWindowGeometryFor:async t=>{assert.equal(t,token);return call('geometryFor',{maximized});},
  windowAction:async()=>{maximized=false;return call('restore',{submitted:true});},
  restoreWindowFor:async t=>{assert.equal(t,token);maximized=false;return call('restoreFor',{submitted:true});},
  trackWindowDrag:async options=>call('drag',{stopped:true,reason:'button_not_down_or_unavailable',effectState:'not_started',dragCompleted:false,options}),
  trackWindowDragFor:async(t,options)=>{assert.equal(t,token);return call('dragFor',{stopped:true,reason:'update_limit',effectState:'unknown',dragCompleted:false,options});},
 };
 return {ctx:{input,signal,desktop},calls,maximize:()=>{maximized=true;}};
}
test('普通示例只进行一次有界调整，不注入按钮或声称完成',async()=>{
 const h=harness(),r=await main(h.ctx);assert.deepEqual(h.calls,['original','pointing','geometry','drag']);
 assert.equal(r.effectState,'not_started');assert.equal(r.dragCompleted,false);assert.equal(r.result.options.maxDurationMs,1000);assert.equal(r.result.options.maxUpdates,32);
});
test('显式B使用同一For租约，默认不交接，不回退A',async()=>{
 const stopped=harness(undefined,true);assert.equal((await main(stopped.ctx)).status,'different_pointing_root');assert(!stopped.calls.includes('handoff'));
 const h=harness({mode:'resize',handoff:true,restore:true},true);h.maximize();const r=await main(h.ctx);
 assert.deepEqual(h.calls,['original','pointing','handoff','geometryFor','restoreFor','geometryFor','dragFor']);assert.equal(r.route,'captured');assert.equal(r.effectState,'unknown');
});
test('非法参数、取消和unknown都不重试或自动继续',async()=>{
 const bad=harness({mode:'move',handoff:false,restore:false,extra:1});await assert.rejects(()=>main(bad.ctx),{code:'invalid_input'});assert.equal(bad.calls.length,0);
 const cancelled=harness();cancelled.ctx.signal.aborted=true;await assert.rejects(()=>main(cancelled.ctx),{code:'cancelled'});assert.equal(cancelled.calls.length,0);
 const h=harness();h.ctx.desktop.trackWindowDrag=async()=>{h.calls.push('drag');throw Object.assign(Error('private'),{code:'timeout',effectState:'unknown'});};
 await assert.rejects(()=>main(h.ctx),e=>e.code==='timeout'&&e.effectState==='unknown'&&!e.message.includes('private'));assert.equal(h.calls.filter(x=>x==='drag').length,1);
});
