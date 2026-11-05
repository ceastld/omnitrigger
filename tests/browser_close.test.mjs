import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import closeTab from '../examples/actions/browser-close/close-tab.mjs';

const base = { profile: 'work', windowId: 4, tabId: 7 };
const window = { id: 4, type: 'normal' };
const tab = { id: 7, windowId: 4 };
function fixture({ input = base, profiles = [{ browser_instance_id: 'work' }], replies = [window, tab, undefined], onProfiles } = {}) {
  const calls = [];
  const ctx = { input: structuredClone(input), browser: {
    async profiles(options) { calls.push(['profiles', options]); await onProfiles?.(ctx); return profiles; },
    async call(method, args, options) {
      calls.push(structuredClone([method, args, options]));
      assert.ok(replies.length, `额外调用 ${method}`);
      const reply = replies.shift(); if (reply instanceof Error) throw reply; return reply;
    },
  } };
  return { ctx, calls };
}
const names = f => f.calls.map(c => c[0]);
const fault = (effectState, code = 'timeout') => Object.assign(new Error('私有标题和网址'), { code, effectState, requiredPermissions: ['tabs'] });

test('动作入口和调用模板身份匹配', async () => {
  const manifest = JSON.parse(await readFile(new URL('../examples/actions/browser-close/action.json', import.meta.url)));
  const call = JSON.parse(await readFile(new URL('../examples/actions/browser-close/call.json', import.meta.url)));
  assert.equal(manifest.id, call.package_id);
  assert.equal(manifest.actions[0].id, call.action_id);
  assert.equal(manifest.actions[0].version, call.action_version);
  assert.equal(manifest.actions[0].runtime, 'node');
  assert.equal(manifest.actions[0].single_instance, true);
  assert.equal((await import(new URL(`../examples/actions/browser-close/${manifest.actions[0].entry}`, import.meta.url))).default, closeTab);
});

test('明确标签无 URL 权限也可核验，只关闭一次并接受 void 回执', async () => {
  const f = fixture(); const result = await closeTab(f.ctx);
  assert.deepEqual(names(f), ['profiles', 'windows.get', 'tabs.get', 'tabs.remove']);
  assert.deepEqual(f.calls[3], ['tabs.remove', [7], { profile: 'work', timeoutMs: 10000 }]);
  assert.deepEqual(result, { status: 'completed', operation: 'close-tab', profile: 'work', target: { tabId: 7, windowId: 4 },
    steps: [{ method: 'tabs.remove', tabId: 7, windowId: 4 }], closeAcknowledged: true });
});

test('支持 popup 和显式数字 0，不查询活动标签', async () => {
  const f = fixture({ input: { ...base, windowId: 0, tabId: 0 }, replies: [{ id: 0, type: 'popup' }, { id: 0, windowId: 0 }, undefined] });
  assert.equal((await closeTab(f.ctx)).closeAcknowledged, true);
  assert.deepEqual(f.calls[3][1], [0]);
});

test('首次异步调用前固定 profile、ID 与期限，多配置不改目标', async () => {
  const f = fixture({ input: { ...base, timeoutMs: 1234 }, profiles: [{ browser_instance_id: 'other' }, { browser_instance_id: 'work' }],
    onProfiles: ctx => Object.assign(ctx.input, { profile: 'other', windowId: 8, tabId: 9, timeoutMs: 1 }) });
  assert.equal((await closeTab(f.ctx)).target.tabId, 7);
  assert.ok(f.calls.slice(1).every(c => c[2].profile === 'work' && c[2].timeoutMs === 1234));
});

for (const [label, input] of Object.entries({ null: null, array: [], empty: {}, profile: { ...base, profile: ' ' },
  missingProfile: { windowId: 4, tabId: 7 }, sentinel: { ...base, tabId: -1 }, unsafe: { ...base, windowId: Number.MAX_SAFE_INTEGER + 1 },
  fractional: { ...base, tabId: 1.2 }, string: { ...base, tabId: '7' }, unknown: { ...base, force: true },
  timeout: { ...base, timeoutMs: 60001 }, zeroTimeout: { ...base, timeoutMs: 0 } })) {
  test(`无效输入 ${label} 在任何调用前拒绝`, async () => {
    const f = fixture({ input }); await assert.rejects(closeTab(f.ctx), e => e.code === 'invalid_input' && e.effectState === 'not_started');
    assert.deepEqual(f.calls, []);
  });
}

for (const [label, profiles, code] of [['missing', [], 'profile_unavailable'], ['other', [{ browser_instance_id: 'other' }], 'profile_unavailable'],
  ['null', null, 'invalid_response'], ['invalid', [{}], 'invalid_response'],
  ['duplicate', [{ browser_instance_id: 'work' }, { browser_instance_id: 'work' }], 'invalid_response']]) {
  test(`配置 ${label} 不选择其他连接`, async () => {
    const f = fixture({ profiles }); await assert.rejects(closeTab(f.ctx), e => e.code === code);
    assert.deepEqual(names(f), ['profiles']);
  });
}

for (const [label, replies] of [['windowMissing', [null]], ['windowChanged', [{ ...window, id: 8 }]],
  ['windowType', [{ ...window, type: 'devtools' }]], ['tabMissing', [window, null]],
  ['tabMoved', [window, { ...tab, windowId: 8 }]], ['otherTab', [window, { ...tab, id: 8 }]]]) {
  test(`目标 ${label} 不派发关闭`, async () => {
    const f = fixture({ replies }); await assert.rejects(closeTab(f.ctx), e => e.code === 'target_changed');
    assert.ok(!names(f).includes('tabs.remove'));
  });
}

test('前置权限错误原样抛出，不伪造关闭回执', async () => {
  const error = fault('not_started', 'permission_required'); const f = fixture({ replies: [window, error] });
  await assert.rejects(closeTab(f.ctx), e => e === error);
  assert.ok(!names(f).includes('tabs.remove'));
});

for (const [effect, status] of [['not_started', 'failed'], ['partial', 'partial'], ['unknown', 'unknown'], [undefined, 'unknown'], ['invalid', 'unknown']]) {
  test(`关闭效果 ${effect} 保留 ${status}，不重试或恢复`, async () => {
    const f = fixture({ replies: [window, tab, fault(effect)] }); const result = await closeTab(f.ctx);
    assert.equal(result.status, status); assert.equal(result.closeAcknowledged, false); assert.deepEqual(result.steps, []);
    assert.equal(result.error.method, 'tabs.remove'); assert.deepEqual(result.error.required_permissions, ['tabs']);
    assert.equal(f.calls.length, 4); assert.equal(names(f).filter(m => m === 'tabs.remove').length, 1);
    assert.ok(!JSON.stringify(result).includes('私有'));
  });
}
