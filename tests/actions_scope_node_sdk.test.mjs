import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import vm from 'node:vm';
import test from 'node:test';

const require=createRequire(new URL('../tools/script-sdk-check/package.json',import.meta.url));
const ts=require('typescript');
const source=readFileSync(new URL('../crates/omnitrigger-actions/runtime/host.mjs',import.meta.url),'utf8');
const shared=readFileSync(new URL('../crates/omnitrigger-actions/runtime/actions-api.js',import.meta.url),'utf8');
const ast=ts.createSourceFile('host.mjs',source,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);
assert.equal(ast.parseDiagnostics.length,0);
const factories=[];
const nativeCalls=[];
function visit(node){
  if(ts.isPropertyAssignment(node)&&node.name.getText(ast)==='actions')factories.push(node.initializer.getText(ast));
  if(ts.isFunctionDeclaration(node)&&node.name?.text==='nativeCall')nativeCalls.push(node.getText(ast));
  ts.forEachChild(node,visit);
}
visit(ast);
assert.equal(factories.length,1);
assert.equal(nativeCalls.length,1);
function make(scope){
  const calls=[];
  const context=vm.createContext({request:{actions_scope:scope,invocation:{invocation_id:'999',binding_id:'777'}},
    nativeCall:(method,params)=>{calls.push({method,params});return Promise.resolve({stoppedCount:1,effectState:'unknown'});}});
  const api=vm.runInContext(`${shared}\n(${factories[0]});`,context);
  return {api,calls,context};
}

test('Node宿主的实际actions属性返回Promise，只把预算交给原生传输',async()=>{
  const {api,calls,context}=make(true);
  const result=api.stopOtherInstances();
  assert.equal(typeof result.then,'function');
  assert.deepEqual(JSON.parse(JSON.stringify(await result)),{stoppedCount:1,effectState:'unknown'});
  assert.deepEqual(JSON.parse(JSON.stringify(calls)),[{method:'actions.stop_other_instances',params:{timeout_ms:500}}]);
  assert.equal(vm.runInContext('typeof request.invocation.invocation_id',context),'string');
  assert(Object.isFrozen(api));
});

test('Node宿主无作用域时有方法并异步拒绝，可读身份不授权传输',async()=>{
  for(const scope of [false,undefined,'true']){
    const {api,calls}=make(scope);
    await assert.rejects(api.stopOtherInstances(),error=>error.code==='actions_unavailable'&&error.effectState==='not_started'&&error.method==='actions.stop_other_instances');
    assert.equal(calls.length,0);
  }
});

test('仅有动作作用域不绕过其余现有上下文守卫，拒绝时零传输',async()=>{
  let transports=0;
  const context=vm.createContext({request:{native:{port:1,token:'synthetic'},native_context_available:false},
    controller:new AbortController(),nativeMethods:new Set(['fs.read_directory','ui.choose','browser.call','clipboard.read','window.geometry']),
    net:{createConnection(){transports++;throw Error('不应传输');}},
    fault:(code,message)=>Object.assign(new Error(message),{code}),checkCancelled:()=>{}});
  const invoke=vm.runInContext(`${nativeCalls[0]}\nnativeCall;`,context);
  for(const [method,code] of [['fs.read_directory','fs_unavailable'],['ui.choose','ui_unavailable'],['browser.call','desktop_unavailable'],['clipboard.read','desktop_unavailable'],['window.geometry','native_bridge_unavailable']]){
    await assert.rejects(invoke(method,{}),error=>error.code===code&&error.effectState==='not_started'&&error.method===method);
  }
  assert.equal(transports,0);
});
