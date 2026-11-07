import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const shared=readFileSync(new URL('./actions-api.js',import.meta.url),'utf8');
const state=readFileSync(new URL('./state-api.js',import.meta.url),'utf8');
const create=vm.runInThisContext(`${shared}\ncreateActionsApi;`);
const receipt=count=>({stoppedCount:count,effectState:count===0?'not_started':'unknown'});
const error=(code,effect)=>value=>value.code===code&&value.effectState===effect&&value.method==='actions.stop_other_instances';

test('同步与异步共用严格预算与回执，不提供任何身份字段',async()=>{
  for(const asyncMode of [false,true]){
    const calls=[];const api=create((method,params)=>{calls.push({method,params});return receipt(2);},asyncMode);
    const pending=api.stopOtherInstances();assert.equal(typeof pending?.then==='function',asyncMode);
    const result=await pending;assert.deepEqual(result,receipt(2));assert(Object.isFrozen(result));assert(Object.isFrozen(api));
    assert.deepEqual(calls,[{method:'actions.stop_other_instances',params:{timeout_ms:500}}]);
    await api.stopOtherInstances({timeoutMs:1});await api.stopOtherInstances({timeoutMs:2000});
    assert.deepEqual(calls.slice(1).map(call=>call.params.timeout_ms),[1,2000]);
  }
});

test('拒绝身份、未知字段、字符串、越界及访问器，零传输',()=>{
  let calls=0,reads=0;const api=create(()=>{calls++;return receipt(0);});
  const getter={};Object.defineProperty(getter,'timeoutMs',{get(){reads++;return 500;}});
  for(const options of [null,[],{invocation_id:'1'},{binding_id:'1'},{package_id:'x'},{action_id:'x'},{timeoutMs:'500'},
    {timeoutMs:0},{timeoutMs:2001},{timeoutMs:1.5},{timeoutMs:undefined},getter,{[Symbol('id')]:true}]){
    assert.throws(()=>api.stopOtherInstances(options),error('invalid_input','not_started'));
  }
  assert.throws(()=>api.stopOtherInstances({},{}),error('invalid_input','not_started'));
  assert.equal(calls,0);assert.equal(reads,0);
});

test('空集合明确not_started，停止集合unknown；坏回执保守unknown',()=>{
  assert.deepEqual(create(()=>receipt(0)).stopOtherInstances(),receipt(0));
  for(const value of [null,{},receipt(4),{...receipt(1),effectState:'not_started'},{...receipt(0),effectState:'unknown'},
    {...receipt(1),extra:true},Promise.resolve(receipt(1))]){
    assert.throws(()=>create(()=>value).stopOtherInstances(),error('invalid_response','unknown'));
  }
  let reads=0;
  const accessor={effectState:'unknown'};
  Object.defineProperty(accessor,'stoppedCount',{get(){reads++;return 1;}});
  assert.throws(()=>create(()=>accessor).stopOtherInstances(),error('invalid_response','unknown'));
  assert.equal(reads,0);
});

test('取消、超时、不可用按结构化错误返回且不重试',async()=>{
  for(const asyncMode of [false,true])for(const [code,effect] of [['stop_timeout','unknown'],['cancelled','unknown'],['action_policy_conflict','not_started'],['actions_unavailable','not_started']]){
    let calls=0;const api=create(()=>{calls++;throw{code,effectState:effect,message:'不得回显调用方内容'};},asyncMode);
    if(asyncMode)await assert.rejects(api.stopOtherInstances(),error(code,effect));else assert.throws(()=>api.stopOtherInstances(),error(code,effect));
    assert.equal(calls,1);
  }
});

function script(scope,native=true,nativeContextAvailable=true){
  const calls=[];
  const context=vm.createContext({ctx:{input:{},invocation:{invocation_id:'999',binding_id:'777'},__nativeContext:native?{connection:{port:1,token:'synthetic'},actions_scope:scope,native_context_available:nativeContextAvailable}:null},
    __hostCall:text=>{const request=JSON.parse(text);calls.push(request);return JSON.stringify({ok:true,value:request.kind==='read_clipboard'?'synthetic clipboard':receipt(1)});},
    createDesktopApi:()=>({fs:{},desktop:{},ui:{},clipboard:{}}),createProcessCaptureApi:()=>({capture(){throw new Error('禁止采集');}}),
    createBrowserApi:()=>({}),createClipboardHistoryApi:()=>({})});
  const sdk=readFileSync(new URL('../../../tools/js-engine-probe/src/sdk.js',import.meta.url),'utf8');
  vm.runInContext(`${shared}\n${state}\n${sdk}`,context);
  return {context,calls};
}

test('完整同步SDK提供真实方法，隐藏作用域元数据并只发预算',()=>{
  const {context,calls}=script(true);
  const value=JSON.parse(vm.runInContext("JSON.stringify(ctx.actions.stopOtherInstances())",context));
  assert.deepEqual(value,receipt(1));assert.equal(calls.length,1);
  assert.equal(calls[0].kind,'native');assert.equal(calls[0].method,'actions.stop_other_instances');
  assert.deepEqual(calls[0].params,{timeout_ms:500});
  assert.equal(vm.runInContext('ctx.__actionsScope===undefined && ctx.__nativeContext===undefined && Object.isFrozen(ctx.actions)',context),true);
});

test('独立CLI即使带可读invocation元数据也不可伪造服务作用域',()=>{
  for(const native of [false,true]){
    const {context,calls}=script(false,native);
    assert.throws(()=>vm.runInContext('ctx.actions.stopOtherInstances()',context),error('actions_unavailable','not_started'));
    assert.equal(calls.length,0);
  }
});

test('只有动作作用域的服务调用保持原独立剪贴板路径，动作控制仍走桥接',()=>{
  const {context,calls}=script(true,true,false);
  assert.equal(vm.runInContext('ctx.clipboard.readText()',context),'synthetic clipboard');
  assert.deepEqual(JSON.parse(vm.runInContext('JSON.stringify(ctx.actions.stopOtherInstances())',context)),receipt(1));
  assert.deepEqual(calls.map(call=>call.kind),['read_clipboard','native']);
  assert.equal(calls[1].method,'actions.stop_other_instances');
});
