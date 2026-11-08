import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
import ts from 'typescript';
const current=readFileSync(new URL('../../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const legacyCaptured="function capturedWindow(args, count, method, operation, validate=value=>value) {\n    if(args.length!==count) invalid(method,'显式窗口接口的参数数量无效。');\n    const target=scopedTarget(args[0],method);\n    const failure=cause=>{\n      let code=typeof cause?.code==='string'&&/^[a-z_]{1,64}$/.test(cause.code)?cause.code:'native_bridge_failed';\n      // 专用类名拒绝只有同一读取方法的not_started才有证明力；不把坏字段正规化成可静默拒绝。\n      if(code==='window_class_blocked'&&(cause?.method!==method||cause?.effectState!=='not_started'))code='invalid_response';\n      throw Object.assign(new Error('显式窗口租约调用未获确认；不会自动重试。'),{code,method,\n        effectState:method==='window.geometry'||cause?.effectState==='not_started'?'not_started':'unknown'});\n    };\n    const api=createDesktopApi((inner,input)=>{\n      if(inner!==method) invalid(method,'显式窗口接口不允许改选操作。');\n      return invoke(method,{...input,target});\n    });\n    try{const value=operation(api.desktop);return value&&typeof value.then==='function'?value.then(validate).catch(failure):validate(value);}\n    catch(cause){return failure(cause);}\n  }";
const legacyProperties={
  "getWindowGeometryFor": "getWindowGeometryFor: (...args) => capturedWindow(args,1,'window.geometry',desktop=>desktop.getWindowGeometry())",
  "setWindowRectFor": "setWindowRectFor: (...args) => capturedWindow(args,3,'window.set_rect',desktop=>desktop.setWindowRect(args[1],args[2]))",
  "restoreWindowFor": "restoreWindowFor: (...args) => capturedWindow(args,1,'window.action',desktop=>desktop.windowAction('restore'),value=>{\n      if(!scopedRecord(value,['submitted'])||value.submitted!==true) throw Object.assign(new Error('还原回执无效。'),{code:'invalid_response',effectState:'unknown'});\n      return {submitted:true};\n    })",
  "toggleMaximizeWindowFor": "toggleMaximizeWindowFor: (...args) => capturedWindow(args,1,'window.action',desktop=>desktop.windowAction('toggle_maximize'),value=>{\n      if(!scopedRecord(value,['submitted'])||value.submitted!==true) throw Object.assign(new Error('最大化切换回执无效。'),{code:'invalid_response',effectState:'unknown'});\n      return {submitted:true};\n    })",
  "trackWindowDragFor": "trackWindowDragFor: (...args) => capturedWindow(args,2,'window.drag',desktop=>desktop.trackWindowDrag(args[1]))"
};
const ast=ts.createSourceFile('current.js',current,ts.ScriptTarget.ES2022,true,ts.ScriptKind.JS);
const factory=ast.statements.find(n=>ts.isFunctionDeclaration(n)&&n.name?.text==='createDesktopApi');
const fn=factory.body.statements.find(n=>ts.isFunctionDeclaration(n)&&n.name?.text==='capturedWindow');
const desktop=factory.body.statements.filter(ts.isVariableStatement).flatMap(n=>[...n.declarationList.declarations]).find(n=>n.name.getText(ast)==='desktop').initializer.arguments[0];
const edits=[{start:fn.getStart(ast),end:fn.end,text:legacyCaptured}];
for(const [name,text] of Object.entries(legacyProperties)){const node=desktop.properties.find(n=>n.name?.getText(ast)===name);assert(node);edits.push({start:node.getStart(ast),end:node.end,text});}
let legacy=current;for(const edit of edits.sort((a,b)=>b.start-a.start))legacy=legacy.slice(0,edit.start)+edit.text+legacy.slice(edit.end);
const build=text=>new vm.Script('(function(){'+text+';return createDesktopApi;})()').runInThisContext();
const factories=[build(legacy),build(current)];
assert(!fn.getText(ast).includes('createDesktopApi('));

const target={capture_id:7,context:{window_handle:'202',process_id:20,thread_id:21,process_created_file_time:'18446744073709551615',expires_at_unix_ms:9999999999999}};
const rect={left:100,top:100,width:700,height:500};
const geometry={coordinateSpace:'physical-screen',className:'TestWindow',outerRect:rect,visibleFrameRect:rect,monitorRect:{left:0,top:0,width:1920,height:1080},workArea:{left:0,top:0,width:1920,height:1040},primaryScreen:{width:2560,height:1440},windowDpi:96,maximized:false};
const drag={mode:'move',button:'left',maxDurationMs:2000,maxUpdates:64,snap:null};
const stopped={stopped:true,reason:'update_limit',acceptedUpdates:1,pollSamples:2,effectState:'unknown',dragCompleted:false};
const cases=[
 ['getWindowGeometryFor',[target],geometry],
 ['setWindowRectFor',[target,rect,{frame:'visible'}],{submitted:true,effectState:'unknown'}],
 ['restoreWindowFor',[target],{submitted:true}],
 ['toggleMaximizeWindowFor',[target],{submitted:true}],
 ['trackWindowDragFor',[target,drag],stopped],
 ['getWindowGeometryFor',[target,{}],geometry],
 ['trackWindowDragFor',[target,{...drag,initialPointer:{x:1,y:2}}],stopped],
 ['getWindowGeometryFor',[{...target,capture_id:0}],geometry],
 ['setWindowRectFor',[target,{...rect,width:0},{frame:'visible'}],{submitted:true,effectState:'unknown'}],
 ['setWindowRectFor',[target,rect,{frame:'auto'}],{submitted:true,effectState:'unknown'}],
 ['trackWindowDragFor',[target,{...drag,maxDurationMs:2001}],stopped],
 ['trackWindowDragFor',[target,{...drag,snap:{bounds:rect,distance:101,waitMs:0,initialElapsedMs:0}}],stopped],
 ['getWindowGeometryFor',[target],{...geometry,className:null}],
 ['restoreWindowFor',[target],{submitted:false}],
 ['toggleMaximizeWindowFor',[target],{submitted:true,extra:1}],
 ['trackWindowDragFor',[target,drag],{...stopped,dragCompleted:true}],
 ['getWindowGeometryFor',[target],null,{code:'window_class_blocked',method:'window.geometry',effectState:'unknown'}],
 ['getWindowGeometryFor',[target],null,{code:'window_class_blocked',method:'window.geometry',effectState:'not_started'}],
 ['setWindowRectFor',[target,rect,{frame:'visible'}],null,{code:'timeout',method:'window.set_rect',effectState:'unknown'}],
];
async function execute(factory,method,args,value,fault,asynchronous){
 const calls=[];const d=factory((method,params)=>{
  calls.push({method,params:structuredClone(params)});
  const run=()=>{if(fault)throw Object.assign(Error('same'),fault);return structuredClone(value);};
  return asynchronous?Promise.resolve().then(run):run();
 }).desktop;
 try{return {ok:true,value:await d[method](...structuredClone(args)),calls};}
 catch(error){return {ok:false,error:{code:error.code,message:error.message,effectState:error.effectState,method:error.method},calls};}
}
for(const asynchronous of [false,true])for(const [index,[method,args,value,fault]] of cases.entries())test(`${asynchronous?'Promise':'同步'} For合同 ${index} ${method}`,async()=>{
 const [before,after]=await Promise.all(factories.map(factory=>execute(factory,method,args,value,fault,asynchronous)));
 assert.deepEqual(after,before);
 if(index<5){assert.equal(after.ok,true);assert.equal(after.calls.length,1);assert.deepEqual(after.calls[0].params.target,target);}
 if(args[1]&&Object.hasOwn(args[1],'initialPointer')){assert.equal(after.ok,true);assert.equal(after.calls.length,1);assert.deepEqual(after.calls[0].params.initial_pointer,{x:1,y:2});}
});

for(const asynchronous of [false,true])test((asynchronous?'Promise':'同步')+' 默认和For均拒绝坐标外的anchor元数据且不发调用',async()=>{
 const calls=[],d=factories[1]((method,params)=>{calls.push({method,params});return asynchronous?Promise.resolve(stopped):stopped;}).desktop;
 for(const invoke of [()=>d.trackWindowDrag({...drag,initialPointer:{x:1,y:2,epoch:1}}),()=>d.trackWindowDragFor(target,{...drag,initialPointer:{x:1,y:2,epoch:1}})])await assert.rejects(async()=>invoke(),e=>e.code==='invalid_input'&&e.effectState==='not_started');
 assert.equal(calls.length,0);
});
