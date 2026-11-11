import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import nodeEntry from '../examples/actions/browser-workflows/duplicate-tab.mjs';
import {main as syncEntry} from '../examples/actions/browser-workflows/duplicate-tab-script.mjs';
import {runWorkflow} from '../examples/actions/browser-workflows/workflows.mjs';
import {cases,observe} from './browser_duplicate_tab_sync.fixtures.mjs';
for(const spec of cases)test(spec.name+'：两个实际入口回执/调用一致并独立核验业务',async()=>{
  const node=await observe(nodeEntry,spec,false),sync=await observe(syncEntry,spec,true);assert.deepEqual(sync,node);
  if(spec.thrown){assert.equal(node.error?.code,spec.thrown);assert.equal(node.calls.filter(c=>c.method==='tabs.duplicate').length,0);}
  else{
    const r=node.result;assert.equal(r?.status,spec.status);assert.equal(r.steps.length,spec.steps);assert.deepEqual(r.target,spec.target);
    if(spec.code){assert.equal(r.error.code,spec.code);assert.equal(r.error.method,spec.fault?.at??(spec.name.startsWith('聚焦回执')?'windows.update':'tabs.duplicate'));
      assert.equal(r.error.effect_state,spec.fault?(spec.steps&&spec.fault.effect==='not_started'?'partial':spec.fault.effect):'partial');}
    else assert.equal(r.error,undefined);
    if(spec.status==='partial'&&spec.fault?.effect==='not_started')assert.equal(r.error.effect_state,'partial');
    if(spec.fault?.permissions)assert.deepEqual(r.error.required_permissions,spec.fault.permissions);
    assert.equal(node.calls.filter(c=>c.method==='tabs.duplicate').length,1);
    assert.equal(node.calls.filter(c=>c.method==='windows.update').length,spec.input.focusWindow===true&&spec.target?1:0);
    if(spec.target)assert.equal(r.steps[0].sourceTabId,spec.input.tabId);
  }
  for(const call of node.calls){
    assert.ok(['profiles','tabs.get','tabs.duplicate','windows.update'].includes(call.method));
    assert.equal(call.options.timeoutMs,spec.input?.timeoutMs??10000);
    if(call.method!=='profiles'){assert.equal(call.options.profile,spec.input?.profile??'own');
      if(call.method==='tabs.get'||call.method==='tabs.duplicate')assert.deepEqual(call.args,[spec.input.tabId]);}
  }
});
test('独立副本及聚焦错误的步骤与目标精确保留',async()=>{
  const find=name=>cases.find(c=>c.name===name);
  const same=await observe(syncEntry,find('副本仍为源ID7-focustrue'),true);assert.deepEqual(same.result.steps,[{method:'tabs.duplicate',sourceTabId:7,windowId:4,tabId:7}]);assert.equal(same.result.target,undefined);assert.equal(same.calls.some(c=>c.method==='windows.update'),false);
  const moved=await observe(syncEntry,find('副本异窗保留真实ID并停止聚焦'),true);assert.deepEqual(moved.result.steps,[{method:'tabs.duplicate',sourceTabId:7,windowId:5,tabId:9}]);assert.equal(moved.result.target,undefined);
  const focus=await observe(syncEntry,find('聚焦回执拒绝3'),true);assert.deepEqual(focus.result.steps,[{method:'tabs.duplicate',sourceTabId:7,windowId:4,tabId:9},{method:'windows.update',windowId:4}]);assert.deepEqual(focus.result.target,{tabId:9,windowId:4});
});
test('既有runWorkflow复制路由与实际Node入口相同，不重复执行',async()=>{
  const spec=cases[1];const routed=await observe(ctx=>runWorkflow(ctx,'duplicate-tab'),spec,false);const direct=await observe(nodeEntry,spec,false);assert.deepEqual(routed,direct);
});
test('包版本/原动作身份/两个调用模板/规范作者类型入口',async()=>{
  const root=new URL('../examples/actions/browser-workflows/',import.meta.url);
  const manifest=JSON.parse(await readFile(new URL('action.json',root),'utf8'));assert.equal(manifest.version,'1.3.4');
  for(const [id,runtime,entry] of [['duplicate-tab','node','duplicate-tab.mjs'],['duplicate-tab-script','script','duplicate-tab-script.mjs']]){
    const action=manifest.actions.find(a=>a.id===id);assert.equal(action.version,1);assert.equal(action.runtime,runtime);assert.equal(action.entry,entry);
    const call=JSON.parse(await readFile(new URL(id+'-call.json',root),'utf8'));assert.equal(call.package_id,manifest.id);assert.equal(call.action_id,id);assert.equal(call.action_version,1);assert.deepEqual(call.input,{profile:'选定的 browser_instance_id',windowId:123,tabId:456,focusWindow:false});
  }
  const project=JSON.parse(await readFile(new URL('jsconfig.json',root),'utf8'));for(const name of ['duplicate-tab.mjs','duplicate-tab-script.mjs','sdk/script-types.d.ts'])assert.ok(project.files.includes(name));
  assert.ok(!project.files.some(name=>name.startsWith('../')));
});
