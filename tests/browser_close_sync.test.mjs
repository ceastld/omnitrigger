import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import closeNode from '../examples/actions/browser-close/close-tab.mjs';
import {main as closeSync} from '../examples/actions/browser-close/close-tab-script.mjs';
import {scenarios,observe,fakeContext,base} from './browser_close_sync.fixtures.mjs';

for(const scenario of scenarios)test(`Node/同步关闭独立契约：${scenario.name}`,async()=>{
  const node=await observe(closeNode,scenario),sync=await observe(closeSync,scenario,{sync:true});
  assert.deepEqual(sync,node,'API trace / JSON receipt / structured error must match');
  assert.equal(sync.kind,scenario.expected.kind);
  assert.deepEqual(sync.calls.map(c=>c.method),scenario.expected.trace);
  if(sync.kind==='throw'){
    assert.equal(sync.error.code,scenario.expected.code);assert.equal(sync.effects.removeAttempts,0);
  }else{
    const receipt=sync.result;
    assert.equal(receipt.status,scenario.expected.status);assert.equal(receipt.operation,'close-tab');
    assert.equal(receipt.profile,'owned');assert.equal(sync.effects.removeAttempts,1);
    assert.deepEqual(JSON.parse(JSON.stringify(receipt)),receipt,'同步回执必须是JSON');
    const input=scenario.input??base;
    assert.deepEqual(receipt.target,{tabId:input.tabId,windowId:input.windowId});
    if(receipt.status==='completed'){
      assert.equal(receipt.closeAcknowledged,true);assert.deepEqual(receipt.steps,[{method:'tabs.remove',tabId:input.tabId,windowId:input.windowId}]);
    }else{assert.equal(receipt.closeAcknowledged,false);assert.deepEqual(receipt.steps,[]);}
    assert.ok(!JSON.stringify(receipt).includes('合成私有'));
    assert.ok(sync.calls.slice(1).every(c=>c.options.profile==='owned'&&c.options.timeoutMs===(input.timeoutMs??10000)));
  }
});

test('新增同步动作保持原Node身份，入口/调用模板/作者文件完整',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../examples/actions/browser-close/action.json',import.meta.url)));
  const node=manifest.actions.find(a=>a.id==='close-tab'),script=manifest.actions.find(a=>a.id==='close-tab-script');
  assert.deepEqual(node,{id:'close-tab',title:'关闭指定浏览器标签',version:1,runtime:'node',entry:'close-tab.mjs',single_instance:true});
  assert.equal(script.runtime,'script');assert.equal(script.version,1);assert.equal(script.single_instance,true);
  assert.equal(script.entry,'close-tab-script.mjs');assert.equal(manifest.actions.length,2);
  const call=JSON.parse(await readFile(new URL('../examples/actions/browser-close/close-tab-script-call.json',import.meta.url)));
  assert.equal(call.package_id,manifest.id);assert.equal(call.action_id,script.id);assert.equal(call.action_version,script.version);
  const project=JSON.parse(await readFile(new URL('../examples/actions/browser-close/jsconfig.json',import.meta.url)));
  assert.equal(project.compilerOptions.strict,true);assert.equal(project.compilerOptions.checkJs,true);
  for(const name of ['close-tab.mjs','close-tab-script.mjs','close-common.mjs','sdk/script-types.d.ts'])assert.ok(project.files.includes(name));
});

test('同步入口不await或回退Promise API，误传异步profiles时在关闭前拒绝',()=>{
  let removes=0;
  const ctx={input:base,browser:{profiles:()=>Promise.resolve([{browser_instance_id:'owned'}]),call:()=>{removes++;}}};
  assert.throws(()=>closeSync(ctx),e=>e.code==='invalid_response'&&e.method==='browser.profiles');assert.equal(removes,0);
});

test('调用方输入和错误权限引用不会因共享helper被改写或借用',async()=>{
  for(const [main,sync] of [[closeNode,false],[closeSync,true]]){
    const f=fakeContext({},{sync});const before=structuredClone(f.ctx.input);await main(f.ctx);
    assert.deepEqual(f.ctx.input,before);assert.equal(Object.isFrozen(f.ctx.input),false);
    const permissions=['tabs',{nested:['private']}];
    const bad=fakeContext({fault:{method:'tabs.remove',code:'disconnected',effectState:'unknown',requiredPermissions:permissions}},{sync});
    const receipt=await main(bad.ctx);permissions[0]='sessions';permissions.push('bookmarks');
    assert.deepEqual(receipt.error.required_permissions,['tabs']);
  }
});
