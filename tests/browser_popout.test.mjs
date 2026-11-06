import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import popOutTab from '../examples/actions/browser-popout/pop-out-tab.mjs';

const base = { profile: 'work', windowId: 4, tabId: 7 };
const source = { id: 4, type: 'normal', incognito: false };
const tab = { id: 7, windowId: 4, incognito: false };
const popup = { id: 9, type: 'popup', incognito: false };
const moved = { id: 7, windowId: 9, incognito: false };
const fault = (code, effectState, extra = {}) => Object.assign(new Error('秘密网址及标题'), { code, effectState, ...extra });
function fixture({ input = base, profiles = [{ browser_instance_id: 'work' }], replies = [source, tab, popup, moved], onProfiles, onCall } = {}) {
  const calls = [];
  const ctx = {
    input: structuredClone(input),
    browser: {
      async profiles(options) {
        calls.push({ method: 'browser.profiles', options });
        await onProfiles?.(ctx);
        if (profiles instanceof Error) throw profiles;
        return profiles;
      },
      async call(method, args, options) {
        calls.push(structuredClone({ method, args, options }));
        await onCall?.(ctx, method);
        assert.ok(replies.length, `非预期额外调用 ${method}`);
        const reply = replies.shift();
        if (reply instanceof Error) throw reply;
        return reply;
      },
    },
  };
  return { ctx, calls };
}
const names = f => f.calls.map(c => c.method);
const rejected = (f, code) => assert.rejects(popOutTab(f.ctx), e => e.code === code && e.effectState === 'not_started');

test('包入口、版本、独占动作与调用模板可解析', async () => {
  const manifest = JSON.parse(await readFile(new URL('../examples/actions/browser-popout/action.json', import.meta.url)));
  const call = JSON.parse(await readFile(new URL('../examples/actions/browser-popout/call.json', import.meta.url)));
  assert.equal(manifest.id, call.package_id);
  assert.equal(manifest.actions.length, 2);
  assert.equal(manifest.actions[0].id, call.action_id);
  assert.equal(manifest.actions[0].version, call.action_version);
  assert.equal(manifest.actions[0].runtime, 'node');
  assert.equal(manifest.actions[0].single_instance, true);
  const entry = await import(new URL(`../examples/actions/browser-popout/${manifest.actions[0].entry}`, import.meta.url));
  assert.equal(entry.default, popOutTab);
});

test('正常弹出只创建一次窗口，按原标签复查，不需要 URL 或 create.tabs', async () => {
  const f = fixture();
  const result = await popOutTab(f.ctx);
  assert.deepEqual(names(f), ['browser.profiles', 'windows.get', 'tabs.get', 'windows.create', 'tabs.get']);
  assert.deepEqual(f.calls[3], { method: 'windows.create', args: [{ tabId: 7, type: 'popup', focused: false, incognito: false }], options: { profile: 'work', timeoutMs: 10000 } });
  assert.deepEqual(result, { status: 'completed', operation: 'pop-out-tab', profile: 'work', source: { windowId: 4, tabId: 7 }, focusRequested: false,
    steps: [{ method: 'windows.create', tabId: 7, windowId: 9 }], createdWindowId: 9, disposition: 'moved', target: { tabId: 7, windowId: 9 } });
});

test('多配置时只使用明确 profile，所有远程调用保持期限', async () => {
  const f = fixture({ input: { ...base, focusWindow: true, timeoutMs: 4321 }, profiles: [{ browser_instance_id: 'other' }, { browser_instance_id: 'work' }] });
  assert.equal((await popOutTab(f.ctx)).focusRequested, true);
  assert.equal(f.calls[3].args[0].focused, true);
  assert.ok(f.calls.slice(1).every(c => c.options.profile === 'work' && c.options.timeoutMs === 4321));
});

test('首次 API 前快照输入，查询期间的改写不改变目标或聚焦', async () => {
  const f = fixture({ onProfiles: ctx => Object.assign(ctx.input, { profile: 'other', windowId: 50, tabId: 70, focusWindow: true, timeoutMs: 1 }) });
  const result = await popOutTab(f.ctx);
  assert.equal(result.profile, 'work');
  assert.deepEqual(f.calls[1].args, [4]);
  assert.deepEqual(f.calls[2].args, [7]);
  assert.deepEqual(f.calls[4].args, [7]);
  assert.equal(f.calls[3].args[0].focused, false);
  assert.ok(f.calls.slice(1).every(c => c.options.timeoutMs === 10000));
});

for (const [label, input] of Object.entries({
  null: null, array: [], missing: {}, noProfile: { windowId: 4, tabId: 7 },
  blankProfile: { ...base, profile: ' ' }, controlProfile: { ...base, profile: 'work\n' },
  longProfile: { ...base, profile: 'x'.repeat(129) }, windowSentinel: { ...base, windowId: -2 },
  tabSentinel: { ...base, tabId: -1 }, unsafeId: { ...base, tabId: Number.MAX_SAFE_INTEGER + 1 },
  fractional: { ...base, windowId: 1.5 }, stringId: { ...base, tabId: '7' },
  implicitActive: { ...base, active: true }, url: { ...base, url: 'https://secret.example/' },
  badFocus: { ...base, focusWindow: 1 }, timeoutZero: { ...base, timeoutMs: 0 },
  timeoutHigh: { ...base, timeoutMs: 60001 }, timeoutFraction: { ...base, timeoutMs: 1.5 },
})) test(`输入 ${label} 在任何 API 前拒绝`, async () => {
  const f = fixture({ input });
  await rejected(f, 'invalid_input');
  assert.deepEqual(f.calls, []);
});

test('真实数字 0 仅作为明确 ID，不被转换成活动标签', async () => {
  const f = fixture({ input: { ...base, windowId: 0, tabId: 0 }, replies: [{ ...source, id: 0 }, { id: 0, windowId: 0, incognito: false }, popup, { id: 0, windowId: 9, incognito: false }] });
  assert.equal((await popOutTab(f.ctx)).target.tabId, 0);
  assert.deepEqual(f.calls[2].args, [0]);
  assert.equal(f.calls[3].args[0].tabId, 0);
});

for (const [label, profiles, code] of [
  ['empty', [], 'profile_unavailable'], ['missing', [{ browser_instance_id: 'other' }], 'profile_unavailable'],
  ['null', null, 'invalid_response'], ['malformed', [{}], 'invalid_response'],
  ['duplicate', [{ browser_instance_id: 'work' }, { browser_instance_id: 'work' }], 'invalid_response'],
]) test(`配置 ${label} 不回退到其他目标`, async () => {
  const f = fixture({ profiles });
  await rejected(f, code);
  assert.deepEqual(names(f), ['browser.profiles']);
});

for (const [label, value, code] of [
  ['missing', null, 'target_changed'], ['other', { ...source, id: 5 }, 'target_changed'],
  ['popup', { ...source, type: 'popup' }, 'target_changed'],
  ['privacyMissing', { id: 4, type: 'normal' }, 'invalid_response'],
]) test(`源窗口 ${label} 在副作用前停止`, async () => {
  const f = fixture({ replies: [value] });
  await rejected(f, code);
  assert.deepEqual(names(f), ['browser.profiles', 'windows.get']);
});

for (const [label, value] of [['missing', null], ['moved', { ...tab, windowId: 5 }], ['other', { ...tab, id: 8 }]]) {
  test(`源标签 ${label} 在副作用前停止`, async () => {
    const f = fixture({ replies: [source, value] });
    await rejected(f, 'target_changed');
    assert.deepEqual(names(f), ['browser.profiles', 'windows.get', 'tabs.get']);
  });
}

test('显式保留源窗口隐私模式，不读取其他 profile', async () => {
  const f = fixture({ replies: [{ ...source, incognito: true }, { ...tab, incognito: true }, { ...popup, incognito: true }, { ...moved, incognito: true }] });
  assert.equal((await popOutTab(f.ctx)).status, 'completed');
  assert.equal(f.calls[3].args[0].incognito, true);
});

for (const method of ['browser.profiles', 'windows.get', 'tabs.get']) test(`前置 ${method} 失败没有派发创建`, async () => {
  const error = fault('cancelled', 'not_started');
  const f = fixture(method === 'browser.profiles' ? { profiles: error } : { replies: method === 'windows.get' ? [error] : [source, error] });
  await assert.rejects(popOutTab(f.ctx), e => e === error);
  assert.ok(!names(f).includes('windows.create'));
});

for (const [code, effectState, status] of [
  ['permission_required', 'not_started', 'failed'], ['cancelled', 'not_started', 'failed'],
  ['timeout', 'unknown', 'unknown'], ['cancelled', 'unknown', 'unknown'],
  ['bridge_disconnected', undefined, 'unknown'], ['unexpected', 'untrusted', 'unknown'],
]) test(`创建 ${code}/${effectState} 保留 ${status} 且不重试`, async () => {
  const f = fixture({ replies: [source, tab, fault(code, effectState, { requiredPermissions: ['tabs'] })] });
  const result = await popOutTab(f.ctx);
  assert.equal(result.status, status);
  assert.deepEqual(result.steps, []);
  assert.equal(result.error.method, 'windows.create');
  assert.deepEqual(result.error.required_permissions, ['tabs']);
  assert.equal(result.target, undefined);
  assert.equal(names(f).filter(n => n === 'windows.create').length, 1);
  assert.equal(f.calls.length, 4);
  assert.ok(!JSON.stringify(result).includes('秘密'));
});

for (const [label, value, knownId] of [
  ['undefined', undefined], ['noId', { type: 'popup', incognito: false }],
  ['negative', { ...popup, id: -1 }], ['sameSource', { ...popup, id: 4 }],
  ['wrongType', { ...popup, type: 'normal' }, 9], ['wrongPrivacy', { ...popup, incognito: true }, 9],
]) test(`创建回执 ${label} 是部分成功，不猜窗口或回滚`, async () => {
  const f = fixture({ replies: [source, tab, value] });
  const result = await popOutTab(f.ctx);
  assert.equal(result.status, 'partial');
  assert.equal(result.steps.length, 1);
  assert.equal(result.createdWindowId, knownId);
  assert.equal(result.error.code, 'invalid_response');
  assert.equal(result.error.effect_state, 'partial');
  assert.equal(result.target, undefined);
  assert.equal(f.calls.length, 4);
});

for (const [label, value, code] of [
  ['missing', null, 'target_changed'], ['otherTab', { ...moved, id: 8 }, 'target_changed'],
  ['movedAgain', { ...moved, windowId: 10 }, 'target_changed'],
  ['cancelled', fault('cancelled', 'not_started'), 'cancelled'],
  ['timeout', fault('timeout', 'unknown'), 'timeout'],
]) test(`创建后复查 ${label} 保留窗口身份与部分效果`, async () => {
  const f = fixture({ replies: [source, tab, popup, value] });
  const result = await popOutTab(f.ctx);
  assert.equal(result.status, 'partial');
  assert.equal(result.createdWindowId, 9);
  assert.equal(result.steps[0].windowId, 9);
  assert.equal(result.error.method, 'tabs.get');
  assert.equal(result.error.code, code);
  assert.equal(result.target, undefined);
  assert.equal(f.calls.length, 5);
});

test('源窗口及网页内容不会直接泄漏到回执', async () => {
  const secret = 'https://private.example/secret';
  const f = fixture({ replies: [{ ...source, tabs: [{ ...tab, title: secret }] }, { ...tab, url: secret }, { ...popup, tabs: [{ ...moved, title: secret }] }, { ...moved, url: secret }] });
  const result = await popOutTab(f.ctx);
  assert.ok(!JSON.stringify(result).includes(secret));
  assert.ok(f.calls.every(c => !['tabs.query', 'tabs.update', 'tabs.move', 'tabs.remove', 'windows.remove', 'windows.getLastFocused'].includes(c.method)));
});


test('合并差异：来源标签隐私上下文不符时不创建', async () => {
  const f=fixture({replies:[source,{...tab,incognito:true}]});
  await rejected(f,'target_changed');
  assert.deepEqual(names(f),['browser.profiles','windows.get','tabs.get']);
});
test('合并差异：移动后隐私上下文不符保留已创建窗口和partial', async () => {
  const f=fixture({replies:[source,tab,popup,{...moved,incognito:true}]});
  const result=await popOutTab(f.ctx);
  assert.equal(result.status,'partial');assert.equal(result.createdWindowId,9);
  assert.equal(result.target,undefined);assert.equal(result.error.code,'target_changed');
  assert.equal(result.steps.length,1);
  assert.deepEqual(names(f),['browser.profiles','windows.get','tabs.get','windows.create','tabs.get']);
});
