import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {EventEmitter} from 'node:events';
import {readFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import ts from 'typescript';
import {runSizeStep} from '../../examples/actions/window-size-step/flow.mjs';
const hostSource=readFileSync(new URL('../../crates/omnitrigger-actions/runtime/host.mjs',import.meta.url),'utf8');
// The complete formal host is parsed as ESM first. Execute its original nativeCall with a mock socket,
// not a replacement RPC model; no product entry, real net/child/fs or Native is executed.
const syntax=spawnSync(process.execPath,['--check',fileURLToPath(new URL('../../crates/omnitrigger-actions/runtime/host.mjs',import.meta.url))],{encoding:'utf8',windowsHide:true,timeout:10000});
assert.ifError(syntax.error);assert.equal(syntax.status,0,syntax.stderr);
const ast=ts.createSourceFile('host.mjs',hostSource,ts.ScriptTarget.ESNext,true,ts.ScriptKind.JS);
assert.equal(ast.parseDiagnostics.length,0);
function declaration(name){const node=ast.statements.find(s=>ts.isFunctionDeclaration(s)&&s.name?.text===name||ts.isVariableStatement(s)&&s.declarationList.declarations.some(d=>d.name.getText(ast)===name));assert(node,name);return node.getText(ast);}
const formalFactory=new vm.Script(`(createConnection)=>{${declaration('nativeMethods')};${declaration('fault')};${declaration('checkCancelled')};${declaration('nativeCall')};
 const controller=new AbortController(),nativeSockets=new Set();let nativePending,nativePendingMethod,nativeId=0;
 const request={native:{port:1,token:'a'.repeat(64)}};return nativeCall;}`).runInThisContext();
const desktopSource=readFileSync(new URL('../../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const desktopFactory=new vm.Script(`(()=>{${desktopSource};return createDesktopApi;})()`).runInThisContext();
const id=b=>({window_handle:b?'202':'101',process_id:b?20:10,thread_id:b?21:11,process_created_file_time:b?'18446744073709551615':'134000000000000001'});
const original=b=>({...id(b),expires_at_unix_ms:9999999999999});
const token={capture_id:7,context:original(true)};
const rect={left:100,top:100,width:700,height:500};
const geometry=maximized=>({coordinateSpace:'physical-screen',className:'TestWindow',outerRect:rect,visibleFrameRect:rect,monitorRect:{left:0,top:0,width:1920,height:1080},workArea:{left:0,top:0,width:1920,height:1040},primaryScreen:{width:2560,height:1440},windowDpi:96,maximized});
function bridge(captured,shape,phase='initial'){
 const calls=[];let restored=false;
 const makeError=method=>{
  const error={code:'window_class_blocked',message:'不调整桌面或任务栏宿主窗口。',method,effect_state:'not_started'};
  if(shape==='unknown')error.effect_state='unknown';
  if(shape==='wrong-method')error.method='pointer.root';
  if(shape==='missing-method')delete error.method;
  if(shape==='missing-effect')delete error.effect_state;
  if(shape==='null-effect')error.effect_state=null;
  if(shape==='unsupported')error.code='unsupported_target';
  return error;
 };
 class Socket extends EventEmitter{
  setEncoding(){return this}setTimeout(){return this}destroy(){queueMicrotask(()=>this.emit('close'))}
  write(raw){const request=JSON.parse(raw);calls.push(request);
   let reply;
   if(request.method==='target.context')reply={value:original(false)};
   else if(request.method==='pointer.root')reply={value:{sampledAt:'call-time',coordinateSpace:'physical-screen',point:{x:1,y:1},inputSource:{kind:'polled-pointer',injected:null,selfInjected:null},pointingWindow:id(captured),rootWindow:id(captured)}};
   else if(request.method==='target.handoff_pointing_root')reply={value:{requested:true,requestAccepted:true,foregroundConfirmed:true,target:token,reason:null,effectState:'unknown'}};
   else if(request.method==='window.geometry')reply=phase==='initial'||phase==='restore'&&restored?{error:makeError(request.method)}:{value:geometry(phase==='restore')};
   else if(request.method==='window.action'){restored=true;reply={value:{submitted:true}};}
   else if(request.method==='window.set_rect')reply={error:makeError(request.method)};
   else assert.fail(request.method);
   queueMicrotask(()=>this.emit('data',JSON.stringify({id:request.id,ok:!reply.error,...reply})+'\n'));
  }
 }
 const createConnection=()=>{const s=new Socket();queueMicrotask(()=>s.emit('connect'));return s;};
 const nativeCall=formalFactory(createConnection);
 const desktop=desktopFactory(nativeCall).desktop;
 const ctx={desktop,input:{trigger:'AdvancedMouseAction',step:50,targetPolicy:captured?'handoff-pointing-root':'original-only',...(captured?{handoff:{validForMs:5000,confirmTimeoutMs:1000}}:{})},signal:{aborted:false}};
 return {ctx,calls};
}
for(const captured of [false,true]){
 const label=captured?'B':'A';
 test(`Node生产函数+模拟socket ${label}：正常专用码才no-op`,async()=>{
  const h=bridge(captured,'valid');const result=await runSizeStep(h.ctx,true,'increase');
  assert.equal(result.status,'source_class_blocked');assert.equal(result.noOpReason,'exact_platform_class');assert.equal(result.effectState,captured?'unknown':'not_started');
  assert(!h.calls.some(c=>c.method==='window.set_rect'||c.method==='window.action'));
 });
 for(const shape of ['unknown','wrong-method','missing-method','missing-effect','null-effect','unsupported'])test(`Node生产函数+模拟socket ${label}：${shape}不能no-op`,async()=>{
  const h=bridge(captured,shape);
  await assert.rejects(()=>runSizeStep(h.ctx,true,'increase'),error=>error.code=== (shape==='unknown'?(captured?'invalid_response':'window_class_blocked'):shape==='unsupported'?'unsupported_target':'invalid_response'));
  assert(!h.calls.some(c=>c.method==='window.set_rect'));
 });
 for(const phase of ['restore','write'])test(`Node生产函数+模拟socket ${label}：${phase}晚期专用错误仍失败`,async()=>{
  const h=bridge(captured,'valid',phase);
  await assert.rejects(()=>runSizeStep(h.ctx,true,'increase'),error=>error.code==='window_class_blocked');
  assert.equal(h.calls.filter(c=>c.method==='window.action').length,phase==='restore'?1:0);
  assert.equal(h.calls.filter(c=>c.method==='window.set_rect').length,phase==='write'?1:0);
 });
}

// Formal synchronous Script SDK frontend; the Rust wire parser has separate source tests.
const scriptSource=readFileSync(new URL('../../tools/js-engine-probe/src/sdk.js',import.meta.url),'utf8');
new vm.Script(scriptSource,{filename:'formal-complete-script-sdk.js'});
const scriptAst=ts.createSourceFile('sdk.js',scriptSource,ts.ScriptTarget.ESNext,true,ts.ScriptKind.JS);
assert.equal(scriptAst.parseDiagnostics.length,0);
const scriptShared=[desktopSource,...['browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js','state-api.js']
 .map(name=>readFileSync(new URL('../../crates/omnitrigger-actions/runtime/'+name,import.meta.url),'utf8'))].join('\n');
// 执行完整前端及真实工厂；只注入正常应用桥的ctx/globalThis，避免复制私有闭包变量。
// 同一V8 realm保留业务JSON对象的原型，FFI仍仅记录并返回合成JSON。
const scriptFactory=new vm.Script(`(__hostCall)=>{const globalThis={__hostCall};
const ctx={input:{},__localDesktopAllowed:false,__nativeContext:{connection:{port:1,token:'a'.repeat(64)},native_context_available:true,actions_scope:true}};
${scriptShared}\n${scriptSource}\nreturn ctx.desktop;}`).runInThisContext();
for(const captured of [false,true])for(const phase of ['success','class','unknown','restore','write'])test(`Script生产前端+模拟JSON FFI（V8） ${captured?'B':'A'}：${phase}`,()=>{
 const calls=[];let restored=false;
 const desktop=scriptFactory(raw=>{
  const request=JSON.parse(raw);assert.equal(request.kind,'native');calls.push(request);
  let value,error;
  if(request.method==='target.context')value=original(false);
  else if(request.method==='pointer.root')value={sampledAt:'call-time',coordinateSpace:'physical-screen',point:{x:1,y:1},inputSource:{kind:'polled-pointer',injected:null,selfInjected:null},pointingWindow:id(captured),rootWindow:id(captured)};
  else if(request.method==='target.handoff_pointing_root')value={requested:true,requestAccepted:true,foregroundConfirmed:true,target:token,reason:null,effectState:'unknown'};
  else if(request.method==='window.geometry'){
   if(['class','unknown'].includes(phase)||phase==='restore'&&restored)error={code:'window_class_blocked',message:'same',method:request.method,effect_state:phase==='unknown'?'unknown':'not_started'};
   else value=geometry(phase==='restore');
  }else if(request.method==='window.action'){restored=true;value={submitted:true};}
  else if(request.method==='window.set_rect'){
   if(phase==='write')error={code:'window_class_blocked',message:'same',method:request.method,effect_state:'not_started'};
   else value={submitted:true,effectState:'unknown'};
  }else assert.fail(request.method);
  return JSON.stringify(error?{ok:false,error}:{ok:true,value});
 });
 const ctx={desktop,input:{trigger:'AdvancedMouseAction',step:50,targetPolicy:captured?'handoff-pointing-root':'original-only',...(captured?{handoff:{validForMs:5000,confirmTimeoutMs:1000}}:{})},signal:{aborted:false}};
 if(phase==='success')assert.equal(runSizeStep(ctx,false,'increase').status,'size_submitted');
 else if(phase==='class')assert.equal(runSizeStep(ctx,false,'increase').status,'source_class_blocked');
 else assert.throws(()=>runSizeStep(ctx,false,'increase'),e=>e.code===(phase==='unknown'&&captured?'invalid_response':'window_class_blocked'));
 assert.equal(calls.filter(c=>c.method==='window.set_rect').length,['success','write'].includes(phase)?1:0);
});
