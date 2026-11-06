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


test('int32输入边界独立断言：上界保留，超界在任何API前拒绝',()=>{
  const max=executeSync(cases.find(c=>c.name==='int32上界'),syncEntry);
  assert.equal(max.returned.status,'completed');assert.deepEqual(max.returned.target,{tabId:2147483647,windowId:2147483647});
  const invalid=executeSync(cases.find(c=>c.name==='窗口超int32'),syncEntry);
  assert.equal(invalid.thrown.code,'invalid_input');assert.deepEqual(invalid.calls,[]);
});

test('int32创建回执独立断言：已确认效果但超界身份保留partial，不猜target',()=>{
  for(const name of ['创建标签ID超int32','创建窗口ID超int32']){
    const value=executeSync(cases.find(c=>c.name===name),syncEntry);
    assert.equal(value.returned.status,'partial');assert.equal(value.returned.error.code,'invalid_response');
    assert.equal(value.returned.target,undefined);assert.deepEqual(value.returned.steps,[{method:'tabs.create',windowId:4}]);
    assert.equal(value.calls.filter(c=>c.method==='tabs.create').length,1);
  }
});
