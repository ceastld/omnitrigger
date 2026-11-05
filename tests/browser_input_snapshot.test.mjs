import test from 'node:test';
import assert from 'node:assert/strict';
import { runWorkflow } from '../examples/actions/browser-workflows/workflows.mjs';

const initial = {
  'open-url': { profile: 'work', timeoutMs: 2345, windowId: 4, url: 'https://original.example/', activate: false, focusWindow: false },
  'open-bookmark': { profile: 'work', timeoutMs: 2345, windowId: 4, query: 'original', bookmarkId: 'b1', activate: false, focusWindow: false },
  'duplicate-tab': { profile: 'work', timeoutMs: 2345, windowId: 4, tabId: 7, focusWindow: false },
  'restore-session': { profile: 'work', timeoutMs: 2345, sessionId: 'old', sessionKind: 'tab' },
};
const changed = { profile: 'other', timeoutMs: 1, windowId: 8, tabId: 8, url: 'https://changed.example/',
  query: 'changed', bookmarkId: 'b2', activate: true, focusWindow: true, sessionId: 'new', sessionKind: 'tab' };
function fixture(operation, mutateAt) {
  const calls = [];
  const ctx = { input: { ...initial[operation] }, browser: {
    async profiles(options) {
      calls.push(['profiles', options]); mutate('profiles');
      return [{ browser_instance_id: 'work' }, { browser_instance_id: 'other' }];
    },
    async call(method, args, options) {
      calls.push(structuredClone([method, args, options])); mutate(method);
      switch (method) {
        case 'windows.get': return { id: args[0], type: 'normal' };
        case 'tabs.query': return [];
        case 'tabs.get': return { id: args[0], windowId: args[0] === 7 ? 4 : 8 };
        case 'tabs.create': return { id: 90, windowId: args[0].windowId };
        case 'tabs.duplicate': return { id: 91, windowId: args[0] === 7 ? 4 : 8 };
        case 'windows.update': return { id: args[0], type: 'normal' };
        case 'bookmarks.search': return [{ id: 'b1', url: 'https://original.example/' }, { id: 'b2', url: 'https://changed.example/' }];
        case 'sessions.getRecentlyClosed': return [{ tab: { sessionId: 'old' } }, { tab: { sessionId: 'new' } }];
        case 'sessions.restore': return { tab: { id: args[0] === 'old' ? 7 : 8, windowId: args[0] === 'old' ? 4 : 8 } };
        default: assert.fail(`非预期调用 ${method}`);
      }
    },
  } };
  function mutate(method) {
    if (method === mutateAt) for (const key of Object.keys(ctx.input)) ctx.input[key] = changed[key];
  }
  return { ctx, calls };
}

for (const operation of Object.keys(initial)) test(`${operation} 在 profiles 等待期间固定最初校验的所有输入`, async () => {
  const f = fixture(operation, 'profiles');
  const result = await runWorkflow(f.ctx, operation);
  assert.equal(result.status, 'completed');
  assert.equal(result.profile, 'work');
  assert.ok(f.calls.slice(1).every(c => c[2].profile === 'work' && c[2].timeoutMs === 2345));
  assert.ok(!f.calls.some(c => c[0] === 'windows.update'));
  if (operation === 'restore-session') {
    assert.deepEqual(f.calls.find(c => c[0] === 'sessions.restore')[1], ['old']);
    assert.deepEqual(result.target, { tabId: 7, windowId: 4 });
  } else if (operation === 'duplicate-tab') {
    assert.deepEqual(f.calls.find(c => c[0] === 'tabs.duplicate')[1], [7]);
    assert.deepEqual(result.target, { tabId: 91, windowId: 4 });
  } else {
    assert.deepEqual(f.calls.find(c => c[0] === 'tabs.create')[1], [{ windowId: 4, url: 'https://original.example/', active: false }]);
    if (operation === 'open-bookmark') assert.deepEqual(f.calls.find(c => c[0] === 'bookmarks.search')[1], [{ query: 'original' }]);
  }
});

for (const [operation, point, method, args] of [
  ['restore-session', 'sessions.getRecentlyClosed', 'sessions.restore', ['old']],
  ['duplicate-tab', 'tabs.get', 'tabs.duplicate', [7]],
  ['open-bookmark', 'bookmarks.search', 'tabs.create', [{ windowId: 4, url: 'https://original.example/', active: false }]],
  ['open-url', 'tabs.query', 'tabs.create', [{ windowId: 4, url: 'https://original.example/', active: false }]],
]) test(`${operation} 在 ${point} 后仍按原参数派发唯一副作用`, async () => {
  const f = fixture(operation, point); const result = await runWorkflow(f.ctx, operation);
  assert.equal(result.status, 'completed');
  const mutations = f.calls.filter(c => ['tabs.create', 'tabs.duplicate', 'sessions.restore', 'windows.update'].includes(c[0]));
  assert.equal(mutations.length, 1); assert.equal(mutations[0][0], method); assert.deepEqual(mutations[0][1], args);
  assert.ok(f.calls.slice(1).every(c => c[2].profile === 'work' && c[2].timeoutMs === 2345));
});

for (const operation of Object.keys(initial)) test(`${operation} 不改写调用方对象，保持未变化输入的既有行为`, async () => {
  const f = fixture(operation); const before = structuredClone(f.ctx.input);
  const result = await runWorkflow(f.ctx, operation);
  assert.equal(result.status, 'completed'); assert.deepEqual(f.ctx.input, before);
  assert.equal(Object.isFrozen(f.ctx.input), false);
});

test('校验仍在任何 API 前执行，冻结不会把非法输入变成有效目标', async () => {
  for (const operation of Object.keys(initial)) {
    const f = fixture(operation); f.ctx.input.unexpected = true;
    await assert.rejects(runWorkflow(f.ctx, operation), e => e.code === 'invalid_input');
    assert.deepEqual(f.calls, []);
  }
});
