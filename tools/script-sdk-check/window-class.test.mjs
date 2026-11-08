import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const source=readFileSync(new URL('../../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const create=Function(source+'\nreturn createDesktopApi;')();
const rect={left:-800,top:100,width:800,height:600};
const base={coordinateSpace:'physical-screen',className:'TestWindow',outerRect:rect,visibleFrameRect:rect,monitorRect:rect,workArea:rect,primaryScreen:{width:2560,height:1440},windowDpi:144,maximized:false};
const target={capture_id:1,context:{window_handle:'11',process_id:21,thread_id:31,process_created_file_time:'41',expires_at_unix_ms:9999999999999}};
function harness(asynchronous,value,failure){
 const calls=[];
 const invoke=(method,params)=>{calls.push({method,params});if(failure)throw Object.assign(Error('private-origin-text'),failure);return value;};
 const {desktop}=create((method,params)=>asynchronous?Promise.resolve().then(()=>invoke(method,params)):invoke(method,params));
 return {desktop,calls};
}
for(const asynchronous of [false,true])for(const captured of [false,true]){
 const label=(asynchronous?'Node':'Script')+(captured?'显式目标':'原目标');
 const call=h=>captured?h.desktop.getWindowGeometryFor(target):h.desktop.getWindowGeometry();
 test(label+'：类名原字保留，SDK不执行来源类名业务匹配',async()=>{
  for(const className of ['TestWindow','窗口类','😀窗口','a'.repeat(255),'Progman','Custom.WorkerW.Proxy','WorKerW']){
   const h=harness(asynchronous,{...base,className}),pending=call(h);
   assert.equal(Boolean(pending&&typeof pending.then==='function'),asynchronous);
   assert.equal((await pending).className,className);assert.equal(h.calls.length,1);
   assert.deepEqual(h.calls[0].params,captured?{target}:{});
  }
 });
 test(label+'：必需类名的长度、NUL及UTF16边界拒绝且不重试',async()=>{
  for(const className of [undefined,null,0,'','a'.repeat(256),'a\0b','\ud800','\udfff','a\ud800b']){
   const value={...base,className};if(className===undefined)delete value.className;
   const h=harness(asynchronous,value);
   await assert.rejects(async()=>call(h),error=>error.code==='invalid_response'&&error.method==='window.geometry'&&error.effectState==='not_started');
   assert.equal(h.calls.length,1);
  }
 });
}
for(const asynchronous of [false,true]){
 const label=asynchronous?'Node':'Script';
 test(label+'：显式目标精确类拒绝只接受同方法not_started证明',async()=>{
  for(const patch of [{},{method:'window.action'},{effectState:'unknown'},{method:undefined},{effectState:undefined}]){
   const failure={code:'window_class_blocked',method:'window.geometry',effectState:'not_started',...patch};
   const h=harness(asynchronous,null,failure),valid=Object.keys(patch).length===0;
   await assert.rejects(async()=>h.desktop.getWindowGeometryFor(target),error=>error.code===(valid?'window_class_blocked':'invalid_response')&&error.method==='window.geometry'&&error.effectState==='not_started'&&!error.message.includes('private-origin-text'));
   assert.equal(h.calls.length,1);
  }
 });
}
