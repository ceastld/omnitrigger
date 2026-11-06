import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import nodeEntry from '../examples/actions/browser-workflows/restore-session.mjs';
import {main as syncEntry} from '../examples/actions/browser-workflows/restore-session-script.mjs';

const base={profile:'own',sessionId:'closed-tab',sessionKind:'tab',timeoutMs:2345};
const fault=(code,effectState='unknown',extra={})=>Object.assign(new Error('合成私有错误正文'),{code,effectState,...extra});
function fixture(spec={},sync=false){
  const calls=[],input=structuredClone(Object.hasOwn(spec,'input')?spec.input:base),ctx={input,browser:null};
  const mutate=method=>{if(spec.mutateAt===method)Object.assign(input,{profile:'other',sessionId:'changed',sessionKind:'window',timeoutMs:1});};
  const profiles=options=>{calls.push({method:'profiles',options:structuredClone(options)});mutate('profiles');
    if(spec.faultAt==='profiles')throw spec.fault;return Object.hasOwn(spec,'profiles')?spec.profiles:[{browser_instance_id:'own'}];};
  const call=(method,args,options)=>{
    calls.push(structuredClone({method,args,options}));mutate(method);
    if(spec.mutateOptions&&method==='sessions.getRecentlyClosed')Object.assign(options,{profile:'other',timeoutMs:1});
    if(spec.faultAt===method)throw spec.fault;
    if(method==='sessions.getRecentlyClosed')return Object.hasOwn(spec,'sessions')?spec.sessions:[{tab:{sessionId:'newer'}},{tab:{sessionId:'closed-tab'}}];
    if(method==='sessions.restore')return Object.hasOwn(spec,'restored')?spec.restored:{tab:{id:7,windowId:4}};
    assert.fail(`禁止额外方法 ${method}`);
  };
  ctx.browser=sync?{profiles,call}:{profiles:async options=>profiles(options),call:async(...args)=>call(...args)};
  for(const key of ['desktop','clipboard','process','open'])Object.defineProperty(ctx,key,{get(){throw Error('禁止访问 '+key);}});
  return{ctx,calls};
}
async function observe(entry,spec,sync){
  const f=fixture(spec,sync);
  try{const value=entry(f.ctx);if(sync)assert.ok(!value||typeof value.then!=='function','同步入口不得返回Promise');
    return{result:sync?value:await value,calls:f.calls,inputAfter:f.ctx.input};
  }catch(error){return{error:{code:error.code,method:error.method,effectState:error.effectState,...(error.requiredPermissions?{requiredPermissions:error.requiredPermissions}:{})},calls:f.calls,inputAfter:f.ctx.input};}
}
async function same(spec={}){const node=await observe(nodeEntry,spec,false),sync=await observe(syncEntry,spec,true);assert.deepEqual(sync,node);return sync;}
const mutations=value=>value.calls.filter(c=>c.method==='sessions.restore');

test('同步恢复沿用明确旧会话，不回退更新关闭项，直接返回JSON',async()=>{
  const v=await same();assert.deepEqual(v.calls.map(c=>c.method),['profiles','sessions.getRecentlyClosed','sessions.restore']);
  assert.deepEqual(v.calls[1],{method:'sessions.getRecentlyClosed',args:[{maxResults:25}],options:{profile:'own',timeoutMs:2345}});
  assert.deepEqual(v.calls[2].args,['closed-tab']);assert.deepEqual(v.result,{status:'completed',profile:'own',operation:'restore-session',steps:[{method:'sessions.restore',sessionId:'closed-tab',sessionKind:'tab'}],target:{tabId:7,windowId:4}});
});

test('同步恢复窗口保留精确窗口ID，标签/窗口类型不互换',async()=>{
  const input={...base,sessionId:'closed-window',sessionKind:'window'};
  const v=await same({input,sessions:[{window:{sessionId:'closed-window'}}],restored:{window:{id:11}}});assert.deepEqual(v.result.target,{windowId:11});
  const wrong=await same({input,sessions:[{tab:{sessionId:'closed-window'}}]});assert.equal(wrong.error.code,'session_unavailable');assert.equal(mutations(wrong).length,0);
});

test('同步恢复缺失/重复会话或无效列表在副作用前停止',async()=>{
  for(const sessions of [[],[{tab:{sessionId:'other'}}],[{tab:{sessionId:'closed-tab'}},{tab:{sessionId:'closed-tab'}}]]){
    const v=await same({sessions});assert.equal(v.error.code,'session_unavailable');assert.equal(mutations(v).length,0);
  }
  const invalid=await same({sessions:null});assert.equal(invalid.error.code,'invalid_response');assert.equal(mutations(invalid).length,0);
});

test('同步恢复输入校验先于任何API，不把连接session_id或数字tabId当参数回退',async()=>{
  for(const input of [{},null,[],{...base,sessionId:7},{...base,sessionId:''},{...base,sessionKind:'latest'},{...base,timeoutMs:0},{...base,windowId:4},{...base,session_id:'connector-session'}]){
    const v=await same({input});assert.equal(v.error.code,'invalid_input');assert.deepEqual(v.calls,[]);
  }
});

test('同步恢复profile选择沿用既有规则，明确profile不可用不回退',async()=>{
  const absent=await same({profiles:[{browser_instance_id:'other'}]});assert.equal(absent.error.code,'profile_unavailable');assert.equal(absent.calls.length,1);
  const ambiguous=await same({input:{sessionId:'closed-tab',sessionKind:'tab'},profiles:[{browser_instance_id:'own'},{browser_instance_id:'other'}]});assert.equal(ambiguous.error.code,'ambiguous_profile');assert.equal(ambiguous.calls.length,1);
  const one=await same({input:{sessionId:'closed-tab',sessionKind:'tab'}});assert.equal(one.result.profile,'own');
});

test('profiles和最近列表等待点修改原输入，不改变原会话/类型/profile/期限',async()=>{
  for(const mutateAt of ['profiles','sessions.getRecentlyClosed']){
    const v=await same({mutateAt});assert.equal(v.result.status,'completed');assert.equal(v.result.profile,'own');
    assert.deepEqual(v.calls[2].args,['closed-tab']);assert.ok(v.calls.slice(1).every(c=>c.options.profile==='own'&&c.options.timeoutMs===2345));
    assert.equal(v.result.steps[0].sessionKind,'tab');
  }
});

test('sessions权限缺失保留原结构化错误，不申请权限或尝试恢复',async()=>{
  const v=await same({faultAt:'sessions.getRecentlyClosed',fault:fault('permission_required','not_started',{requiredPermissions:['sessions']})});
  assert.equal(v.error.code,'permission_required');assert.equal(v.error.effectState,'not_started');assert.deepEqual(v.error.requiredPermissions,['sessions']);assert.equal(v.calls.length,2);assert.equal(mutations(v).length,0);
});

test('恢复派发not_started/unknown保留原语义，最多派发一次且不补偿',async()=>{
  for(const [effectState,status] of [['not_started','failed'],['unknown','unknown'],['partial','unknown']]){
    const v=await same({faultAt:'sessions.restore',fault:fault('disconnected',effectState)});assert.equal(v.result.status,status);assert.equal(v.result.error.effect_state,effectState);assert.deepEqual(v.result.steps,[]);assert.equal(mutations(v).length,1);
  }
});

test('已确认恢复但身份缺失/超int32时保留partial和会话步骤，不猜target',async()=>{
  for(const restored of [undefined,null,{}, {tab:{id:2147483648,windowId:4}}, {tab:{id:7,windowId:2147483648}}]){
    const v=await same({restored});assert.equal(v.result.status,'partial');assert.equal(v.result.target,undefined);assert.equal(v.result.error.code,'invalid_response');assert.equal(v.result.steps.length,1);assert.equal(mutations(v).length,1);
  }
  const window=await same({input:{...base,sessionId:'closed-window',sessionKind:'window'},sessions:[{window:{sessionId:'closed-window'}}],restored:{window:{id:2147483648}}});assert.equal(window.result.status,'partial');assert.equal(window.result.target,undefined);
});

test('恢复回执0/int32上界有效，不带网页正文、标题或整份API响应',async()=>{
  for(const id of [0,2147483647]){
    const v=await same({restored:{tab:{id,windowId:id,url:'合成私有URL',title:'合成私有标题'}}});assert.equal(v.result.status,'completed');assert.deepEqual(v.result.target,{tabId:id,windowId:id});assert.ok(!JSON.stringify(v.result).includes('合成私有'));
  }
});

test('同步恢复误接异步profiles时在任何副作用前拒绝',()=>{
  let calls=0;assert.throws(()=>syncEntry({input:base,browser:{profiles:()=>Promise.resolve([{browser_instance_id:'own'}]),call:()=>calls++}}),e=>e.code==='invalid_response');assert.equal(calls,0);
});

test('同步恢复真实入口、两种模板与严格canonical工程配置完整',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../examples/actions/browser-workflows/action.json',import.meta.url)));
  const old=manifest.actions.find(a=>a.id==='restore-session'),sync=manifest.actions.find(a=>a.id==='restore-session-script');assert.equal(old.runtime,'node');assert.equal(old.version,1);assert.equal(sync.runtime,'script');assert.equal(sync.version,1);assert.equal(sync.single_instance,true);assert.equal(sync.entry,'restore-session-script.mjs');
  for(const [name,id] of [['restore-session-call.json','restore-session'],['restore-session-script-call.json','restore-session-script']]){
    const call=JSON.parse(await readFile(new URL('../examples/actions/browser-workflows/'+name,import.meta.url)));assert.equal(call.package_id,manifest.id);assert.equal(call.action_id,id);assert.equal(call.action_version,1);
  }
  const project=JSON.parse(await readFile(new URL('../examples/actions/browser-workflows/jsconfig.json',import.meta.url)));assert.equal(project.compilerOptions.strict,true);assert.equal(project.compilerOptions.checkJs,true);assert.ok(project.files.includes('../../../crates/omnitrigger-actions/runtime/script-types.d.ts'));assert.ok(!project.files.includes('sdk/script-types.d.ts'));
});


test('读取API改写传入选项引用，不改变后续恢复的profile和期限',async()=>{
  const v=await same({mutateOptions:true});assert.equal(v.result.status,'completed');
  assert.deepEqual(v.calls[2].options,{profile:'own',timeoutMs:2345});
});
