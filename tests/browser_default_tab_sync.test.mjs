import assert from 'node:assert/strict';
import test from 'node:test';
import {runWorkflow as candidate} from '../examples/actions/browser-workflows/workflows.mjs';
import {main as syncEntry} from '../examples/actions/browser-workflows/default-new-tab-script.mjs';
import {cases,executeSync,executeAsync} from './browser_default_tab_sync.fixtures.mjs';
for(const spec of cases)test(spec.name+'：完整同步与异步驱动一致',async()=>{
  const expected=await executeAsync(spec,ctx=>candidate(ctx,'create-default-tab'));
  const sync=executeSync(spec,syncEntry);assert.deepEqual(sync,expected);
  assert.ok(sync.calls.filter(call=>call.method==='tabs.create').length<=1,'副作用不得自动重试');
  for(const call of sync.calls)if(call.method==='tabs.create')assert.equal(Object.hasOwn(call.args[0],'url'),false);
});
test('关键回执边界独立断言',()=>{
  const value=name=>executeSync(cases.find(c=>c.name===name),syncEntry);
  assert.equal(value('创建身份异常0').returned.status,'partial');
  assert.equal(value('取消-tabs.create-not_started').returned.status,'failed');
  assert.equal(value('取消-tabs.create-unknown').returned.status,'unknown');
  const partial=value('取消-windows.update-not_started').returned;
  assert.equal(partial.status,'partial');assert.equal(partial.error.effect_state,'partial');
  assert.deepEqual(partial.target,{tabId:7,windowId:4});
  assert.equal(value('取消-windows.get-not_started').thrown.code,'cancelled');
  assert.equal(value('后台').returned.addressBarRequested,false);
});
