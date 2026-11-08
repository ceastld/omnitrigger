import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, dirname, basename, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { runWorkflow } from '../examples/actions/browser-workflows/workflows.mjs';

const window = { id: 4, type: 'normal' };
const tab = { id: 7, windowId: 4, url: 'https://example.com/' };
const base = { windowId: 4, url: 'https://example.com/' };
const fault = (code, effectState = 'not_started', requiredPermissions) =>
  Object.assign(new Error('模拟错误'), { code, effectState, requiredPermissions });
function fixture(input = base, replies = [], profiles = [{ browser_instance_id: 'work' }]) {
  const calls = [];
  return {
    input: { ...input }, calls,
    browser: {
      async profiles(options) { calls.push(['profiles', options]); return profiles; },
      async call(method, args, options) {
        calls.push([method, args, options]);
        assert.ok(replies.length, `非预期调用 ${method}`);
        const reply = replies.shift();
        if (reply instanceof Error) throw reply;
        return reply;
      },
    },
  };
}
const names = ctx => ctx.calls.map(call => call[0]);
const rejects = (ctx, operation, code) => assert.rejects(runWorkflow(ctx, operation), error => error.code === code);

test('动作包各入口可导入，版本与运行时明确', async () => {
  const manifest = JSON.parse(await readFile(new URL('../examples/actions/browser-workflows/action.json', import.meta.url)));
  assert.equal(manifest.id, 'examples.browser-workflows');
  for (const action of manifest.actions) {
    assert.equal(action.version, 1);
    const entry = await import(new URL(`../examples/actions/browser-workflows/${action.entry}`, import.meta.url));
    if (action.id === 'create-default-tab-script' || action.id === 'restore-session-script') {
      assert.equal(action.runtime, 'script');
      assert.equal(action.single_instance, true);
      assert.equal(action.entry, action.id === 'create-default-tab-script' ? 'default-new-tab-script.mjs' : 'restore-session-script.mjs');
      assert.equal(typeof entry.main, 'function');
    } else {
      assert.equal(action.runtime, 'node');
      assert.equal(typeof entry.default, 'function');
    }
  }
});

test('输入错误在配置查询和副作用前被拒绝', async () => {
  for (const input of [
    {}, { ...base, windowId: -2 }, { ...base, windowId: Number.MAX_SAFE_INTEGER + 1 },
    { ...base, active: true }, { ...base, activate: false, focusWindow: true },
    { ...base, url: 'javascript:alert(1)' }, { ...base, url: 'https://user:secret@example.com/' },
    { ...base, profile: ' ' }, { ...base, timeoutMs: 0 }, { ...base, tabId: null },
  ]) {
    const ctx = fixture(input);
    await rejects(ctx, 'open-url', 'invalid_input');
    assert.deepEqual(ctx.calls, []);
  }
});

test('多 profile 不默选首个，消失的明确 profile 不回退', async () => {
  const profiles = [{ browser_instance_id: 'work' }, { browser_instance_id: 'personal' }];
  for (const [input, expected, available] of [
    [base, 'ambiguous_profile', profiles],
    [{ ...base, profile: 'missing' }, 'profile_unavailable', profiles],
    [base, 'browser_unavailable', []],
    [base, 'invalid_response', [profiles[0], profiles[0]]],
    [base, 'invalid_response', [{}]],
  ]) {
    const ctx = fixture(input, [], available);
    await rejects(ctx, 'open-url', expected);
    assert.deepEqual(names(ctx), ['profiles']);
  }
});

test('明确 profile 固定到每次调用，创建始终携带目标窗口', async () => {
  const ctx = fixture({ ...base, profile: 'personal', activate: false, timeoutMs: 1234 },
    [window, [], { id: 9, windowId: 4 }], [{ browser_instance_id: 'work' }, { browser_instance_id: 'personal' }]);
  const result = await runWorkflow(ctx, 'open-url');
  assert.equal(result.status, 'completed');
  assert.deepEqual(result.target, { tabId: 9, windowId: 4 });
  assert.equal(result.disposition, 'created');
  assert.deepEqual(ctx.calls[3], ['tabs.create', [{ windowId: 4, url: base.url, active: false }], { profile: 'personal', timeoutMs: 1234 }]);
  assert.ok(ctx.calls.slice(1).every(call => call[2].profile === 'personal'));
});

test('复用规范化精确 URL 并复查身份，激活和窗口聚焦分步确认', async () => {
  const ctx = fixture({ ...base, url: 'https://EXAMPLE.com:443', focusWindow: true }, [window, [tab], tab, { ...tab, active: true }, { ...window, focused: true }]);
  const result = await runWorkflow(ctx, 'open-url');
  assert.equal(result.disposition, 'reused');
  assert.deepEqual(names(ctx), ['profiles', 'windows.get', 'tabs.query', 'tabs.get', 'tabs.update', 'windows.update']);
  assert.deepEqual(ctx.calls[2][1], [{ windowId: 4 }]);
  assert.deepEqual(ctx.calls[4][1], [7, { active: true }]);
  assert.equal(result.steps.length, 2);
});

test('不激活的复用无副作用，不忽略 URL fragment', async () => {
  const ctx = fixture({ ...base, activate: false }, [window, [tab], tab]);
  const result = await runWorkflow(ctx, 'open-url');
  assert.equal(result.status, 'completed');
  assert.deepEqual(result.steps, []);
  const fragment = fixture({ ...base, url: `${base.url}#section`, activate: false }, [window, [tab], { id: 10, windowId: 4 }]);
  assert.equal((await runWorkflow(fragment, 'open-url')).disposition, 'created');
});

test('多个网址匹配必须消歧，明确 tabId 不匹配时不创建替代标签', async () => {
  const other = { ...tab, id: 8 };
  const ambiguous = fixture(base, [window, [tab, other]]);
  await rejects(ambiguous, 'open-url', 'ambiguous_tab');
  assert.equal(ambiguous.calls.length, 3);
  const explicit = fixture({ ...base, tabId: 8, activate: false }, [window, [tab, other], other]);
  assert.equal((await runWorkflow(explicit, 'open-url')).target.tabId, 8);
  const missing = fixture({ ...base, tabId: 9 }, [window, [tab]]);
  await rejects(missing, 'open-url', 'target_changed');
  assert.equal(missing.calls.length, 3);
});

test('网址不可读时明确报告 tabs 权限，防止错误新建', async () => {
  const ctx = fixture(base, [window, [{ id: 7, windowId: 4 }]]);
  await assert.rejects(runWorkflow(ctx, 'open-url'), error => error.code === 'permission_required'
    && error.effectState === 'not_started' && error.requiredPermissions[0] === 'tabs');
  assert.equal(ctx.calls.length, 3);
});

test('pendingUrl 优先，原标签正在离开目标 URL 时不复用', async () => {
  const ctx = fixture(base, [window, [{ ...tab, pendingUrl: 'https://elsewhere.example/' }], { id: 8, windowId: 4 }]);
  assert.equal((await runWorkflow(ctx, 'open-url')).disposition, 'created');
});

test('目标窗口、标签移动或网址变化时在修改前终止', async () => {
  for (const replies of [
    [{ ...window, id: 5 }],
    [window, [{ ...tab, windowId: 5 }]],
    [window, [tab], { ...tab, windowId: 5 }],
    [window, [tab], { ...tab, url: 'https://changed.example/' }],
    [window, [tab], { ...tab, pendingUrl: 'https://changed.example/' }],
  ]) {
    const ctx = fixture(base, replies);
    await rejects(ctx, 'open-url', 'target_changed');
    assert.ok(!names(ctx).includes('tabs.update'));
    assert.ok(!names(ctx).includes('tabs.create'));
  }
});

test('结构异常不当作空列表，列表重复 ID 被拒绝', async () => {
  for (const replies of [[window, null], [window, [tab, tab]], [window, [{ ...tab, id: -1 }]]]) {
    const ctx = fixture(base, replies);
    await rejects(ctx, 'open-url', 'invalid_response');
    assert.equal(ctx.calls.length, 3);
  }
});

test('创建成功后聚焦失败保留新标签 ID、权限与部分成功，不自动重试', async () => {
  const ctx = fixture({ ...base, focusWindow: true }, [window, [], { id: 9, windowId: 4, active: true }, fault('permission_required', 'not_started', ['windows'])]);
  const result = await runWorkflow(ctx, 'open-url');
  assert.equal(result.status, 'partial');
  assert.deepEqual(result.target, { tabId: 9, windowId: 4 });
  assert.deepEqual(result.error, { code: 'permission_required', method: 'windows.update', effect_state: 'partial', required_permissions: ['windows'] });
  assert.equal(result.steps.length, 1);
  assert.equal(names(ctx).filter(name => name === 'tabs.create').length, 1);
});

test('派发后结果未知保留 unknown；明确未开始则 failed', async () => {
  for (const [effect, status] of [['unknown', 'unknown'], ['not_started', 'failed']]) {
    const ctx = fixture(base, [window, [], fault('transport_error', effect)]);
    const result = await runWorkflow(ctx, 'open-url');
    assert.equal(result.status, status);
    assert.equal(result.error.effect_state, effect);
    assert.deepEqual(result.steps, []);
    assert.equal(ctx.calls.length, 4);
  }
});

test('副作用已确认但回执缺少 ID 时不能称作未执行', async () => {
  const ctx = fixture(base, [window, [], undefined]);
  const result = await runWorkflow(ctx, 'open-url');
  assert.equal(result.status, 'partial');
  assert.equal(result.error.code, 'invalid_response');
  assert.equal(result.steps[0].method, 'tabs.create');
});

test('副作用结果落入其他窗口时保留实际返回的精确 ID，停止后续聚焦', async () => {
  for (const [operation, input, replies] of [
    ['open-url', { ...base, focusWindow: true }, [window, [], { id: 20, windowId: 5 }]],
    ['duplicate-tab', { windowId: 4, tabId: 7, focusWindow: true }, [tab, { id: 20, windowId: 5 }]],
  ]) {
    const ctx = fixture(input, replies);
    const result = await runWorkflow(ctx, operation);
    assert.equal(result.status, 'partial');
    assert.equal(result.error.code, 'target_changed');
    assert.equal(result.steps[0].tabId, 20);
    assert.equal(result.steps[0].windowId, 5);
    assert.ok(!names(ctx).includes('windows.update'));
  }
});

test('复制只用指定 ID，源标签移走时不操作其他活动标签', async () => {
  const input = { windowId: 4, tabId: 7 };
  const ctx = fixture(input, [tab, { id: 9, windowId: 4 }]);
  const result = await runWorkflow(ctx, 'duplicate-tab');
  assert.deepEqual(names(ctx), ['profiles', 'tabs.get', 'tabs.duplicate']);
  assert.deepEqual(ctx.calls[2][1], [7]);
  assert.deepEqual(result.target, { tabId: 9, windowId: 4 });
  const moved = fixture(input, [{ ...tab, windowId: 5 }]);
  await rejects(moved, 'duplicate-tab', 'target_changed');
  assert.equal(moved.calls.length, 2);
});

test('复制 undefined 回执报告部分成功，不查询活动标签猜测副本', async () => {
  const ctx = fixture({ windowId: 4, tabId: 7 }, [tab, undefined]);
  const result = await runWorkflow(ctx, 'duplicate-tab');
  assert.equal(result.status, 'partial');
  assert.equal(result.steps[0].sourceTabId, 7);
  assert.deepEqual(names(ctx), ['profiles', 'tabs.get', 'tabs.duplicate']);
});

test('恢复必须给会话 ID 和类型，按 ID 恢复而不是最近关闭', async () => {
  const input = { sessionId: 'closed-tab', sessionKind: 'tab' };
  const ctx = fixture(input, [[{ tab: { sessionId: 'newer-tab' } }, { tab: { sessionId: 'closed-tab' } }], { tab }]);
  const result = await runWorkflow(ctx, 'restore-session');
  assert.deepEqual(ctx.calls[2][1], ['closed-tab']);
  assert.deepEqual(result.target, { tabId: 7, windowId: 4 });
  const kind = fixture({ ...input, sessionKind: 'window' }, [[{ tab: { sessionId: 'closed-tab' } }]]);
  await rejects(kind, 'restore-session', 'session_unavailable');
  const absent = fixture({});
  await rejects(absent, 'restore-session', 'invalid_input');
  assert.deepEqual(absent.calls, []);
});

test('恢复窗口保留窗口身份；sessions 权限错误在副作用前保持结构', async () => {
  const input = { sessionId: 'closed-window', sessionKind: 'window' };
  const ctx = fixture(input, [[{ window: { sessionId: input.sessionId } }], { window: { id: 11 } }]);
  assert.deepEqual((await runWorkflow(ctx, 'restore-session')).target, { windowId: 11 });
  const denied = fixture(input, [fault('permission_required', 'not_started', ['sessions'])]);
  await assert.rejects(runWorkflow(denied, 'restore-session'), error => error.requiredPermissions[0] === 'sessions');
  assert.equal(denied.calls.length, 2);
});

test('书签搜索排除文件夹，多匹配要求明确书签 ID，打开复用同一 URL 流程', async () => {
  const input = { windowId: 4, query: '示例', bookmarkId: 'b', activate: false };
  const nodes = [{ id: 'folder', title: '示例' }, { id: 'a', url: 'https://other.example/' }, { id: 'b', url: base.url }];
  const ctx = fixture(input, [nodes, window, [tab], tab]);
  const result = await runWorkflow(ctx, 'open-bookmark');
  assert.equal(result.bookmarkId, 'b');
  assert.equal(result.disposition, 'reused');
  assert.deepEqual(names(ctx), ['profiles', 'bookmarks.search', 'windows.get', 'tabs.query', 'tabs.get']);
  const multiple = fixture({ windowId: 4, query: '示例' }, [nodes]);
  await rejects(multiple, 'open-bookmark', 'ambiguous_bookmark');
  const missing = fixture(input, [[nodes[0]]]);
  await rejects(missing, 'open-bookmark', 'bookmark_not_found');
});

test('书签非网页 URL 被拒绝；输出摘要不保存标题、URL 或搜索词', async () => {
  const ctx = fixture({ windowId: 4, query: '私有搜索词' }, [[{ id: 'b', title: '私有标题', url: 'javascript:void(0)' }]]);
  await rejects(ctx, 'open-bookmark', 'unsupported_url');
  const valid = fixture({ windowId: 4, query: '私有搜索词' }, [[{ id: 'b', title: '私有标题', url: base.url }], window, [], { id: 8, windowId: 4 }]);
  const output = JSON.stringify(await runWorkflow(valid, 'open-bookmark'));
  assert.ok(!output.includes('私有'));
  assert.ok(!output.includes('example.com'));
});

// 真正 Node 动作宿主与动作入口集成；SDK 对端是隔离 fixture，不代表真实 Chrome。
test('Node 动作宿主实际加载入口，保留部分成功回执与抛出的权限结构', async () => {
  const root = await mkdtemp(join(tmpdir(), 'omnitrigger-browser-workflow-host-'));
  const { writeFile } = await import('node:fs/promises');
  try {
    const sdk = join(root, 'sdk.mjs');
    await writeFile(sdk, `export async function connect() {
      return { close() {}, async profiles() { return [{browser_instance_id:'fixture'}]; },
        async call(method, args) {
          if (method === 'windows.get') return {id:4,type:'normal'};
          if (method === 'tabs.query') return [];
          if (method === 'tabs.create') return {id:12,windowId:4,active:true};
          if (method === 'windows.update' || method === 'sessions.getRecentlyClosed')
            throw Object.assign(new Error('需要权限'),{code:'permission_required',method,effectState:'not_started',requiredPermissions:['sessions']});
          throw new Error('非预期调用');
        }
      };
    }`);
    // 与 node.rs 相同拼接顺序，宿主源码本身不是独立可执行入口。
    const host = join(root, 'host.mjs');
    const runtime = new URL('../crates/omnitrigger-actions/runtime/', import.meta.url);
    const sources = await Promise.all(['desktop-api.js', 'browser-api.js', 'clipboard-history-api.js', 'process-capture-api.js', 'actions-api.js', 'state-api.js', 'host.mjs']
      .map(name => readFile(new URL(name, runtime), 'utf8')));
    await writeFile(host, sources.join('\n'));
    for (const [entry, input, expected] of [
      ['open-url', { ...base, focusWindow: true }, 'ok'],
      ['restore-session', { sessionId: 'closed', sessionKind: 'tab' }, 'error'],
    ]) {
      const response = join(root, `${entry}.json`);
      const packageRoot = fileURLToPath(new URL('../examples/actions/browser-workflows/', import.meta.url));
      const request = {
        api_version: 1, invocation: { invocation_id: entry, binding_id: 'fixture' },
        entry_path: join(packageRoot, `${entry}.mjs`), package_root: packageRoot,
        response_path: response, input, timeout_ms: 5000,
        browser: { sdk_path: sdk, config_path: join(root, 'unused.json') },
      };
      const child = spawn(process.execPath, [host], { stdio: ['pipe', 'ignore', 'pipe'] });
      let stderr = '';
      child.stderr.on('data', data => { stderr += data; });
      const completion = new Promise((resolve, reject) => {
        child.once('error', reject);
        child.once('exit', code => code === 0 ? resolve() : reject(new Error(`宿主 ${code}: ${stderr}`)));
      });
      child.stdin.write(`${JSON.stringify(request)}\n`);
      // 宿主保持 stdin 到响应完成；检测文件后关闭，避免 EOF 提前取消动作。
      const timeout = setTimeout(() => child.kill(), 8000);
      const interval = setInterval(async () => {
        try { await readFile(response); child.stdin.end(); } catch { /* 尚未响应 */ }
      }, 10);
      try { await completion; } finally { clearTimeout(timeout); clearInterval(interval); }
      const result = JSON.parse(await readFile(response));
      assert.equal(result.status, expected);
      if (expected === 'ok') {
        assert.equal(result.output.status, 'partial');
        assert.deepEqual(result.output.target, { tabId: 12, windowId: 4 });
      } else {
        assert.equal(result.error.effect_state, 'not_started');
        assert.deepEqual(result.error.required_permissions, ['sessions']);
      }
    }
  } finally {
    assert.equal(dirname(resolve(root)), resolve(tmpdir()));
    assert.ok(basename(root).startsWith('omnitrigger-browser-workflow-host-'));
    await rm(root, { recursive: true, force: true });
  }
});

test('复用标签激活回执缺失或未确认时保留 partial 并停止聚焦', async () => {
  for (const reply of [undefined, { ...tab, active: false }, tab]) {
    const ctx = fixture({ ...base, focusWindow: true }, [window, [tab], tab, reply]);
    const result = await runWorkflow(ctx, 'open-url');
    assert.equal(result.status, 'partial');
    assert.deepEqual(result.target, { tabId: 7, windowId: 4 });
    assert.deepEqual(result.error, { code: 'activation_unconfirmed', method: 'tabs.update', effect_state: 'partial' });
    assert.deepEqual(result.steps, [{ method: 'tabs.update', tabId: 7, windowId: 4 }]);
    assert.deepEqual(names(ctx), ['profiles', 'windows.get', 'tabs.query', 'tabs.get', 'tabs.update']);
  }
});

test('复用标签激活返回错误身份或无效结构时停止聚焦且不重试', async () => {
  for (const [reply, code] of [
    [null, 'invalid_response'], [{ ...tab, id: -1, active: true }, 'invalid_response'],
    [{ ...tab, id: 8, active: true }, 'target_changed'],
    [{ ...tab, windowId: 5, active: true }, 'target_changed'],
  ]) {
    const ctx = fixture({ ...base, focusWindow: true }, [window, [tab], tab, reply]);
    const result = await runWorkflow(ctx, 'open-url');
    assert.equal(result.status, 'partial');
    assert.equal(result.error.code, code);
    assert.equal(result.error.method, 'tabs.update');
    assert.equal(result.error.effect_state, 'partial');
    assert.equal(result.steps.length, 1);
    assert.deepEqual(names(ctx), ['profiles', 'windows.get', 'tabs.query', 'tabs.get', 'tabs.update']);
  }
});


test('新建要求激活但回执未确认时保留标签ID和partial，不继续聚焦', async () => {
  for (const active of [false, undefined]) {
    const created = { id: 9, windowId: 4, ...(active === undefined ? {} : { active }) };
    const ctx = fixture({ ...base, focusWindow: true }, [window, [], created]);
    const result = await runWorkflow(ctx, 'open-url');
    assert.equal(result.status, 'partial');
    assert.deepEqual(result.target, { tabId: 9, windowId: 4 });
    assert.equal(result.disposition, 'created');
    assert.deepEqual(result.error, { code: 'activation_unconfirmed', method: 'tabs.create', effect_state: 'partial' });
    assert.deepEqual(names(ctx), ['profiles', 'windows.get', 'tabs.query', 'tabs.create']);
  }
});

test('聚焦回执必须确认指定窗口与focused，失败保留partial且不重试', async () => {
  for (const [reply, code] of [[undefined, 'invalid_response'], [null, 'invalid_response'],
    [{ id: 5, focused: true }, 'target_changed'], [{ id: 4, focused: false }, 'focus_unconfirmed'],
    [{ id: 4 }, 'focus_unconfirmed']]) {
    const ctx = fixture({ ...base, focusWindow: true }, [window, [tab], tab, { ...tab, active: true }, reply]);
    const result = await runWorkflow(ctx, 'open-url');
    assert.equal(result.status, 'partial');
    assert.deepEqual(result.target, { tabId: 7, windowId: 4 });
    assert.equal(result.error.code, code);
    assert.equal(result.error.method, 'windows.update');
    assert.equal(result.error.effect_state, 'partial');
    assert.equal(result.steps.length, 2);
    assert.equal(names(ctx).filter(method => method === 'windows.update').length, 1);
  }
});

test('新建激活和明确窗口聚焦均确认时完成', async () => {
  const ctx = fixture({ ...base, focusWindow: true }, [window, [], { id: 9, windowId: 4, active: true }, { id: 4, focused: true }]);
  const result = await runWorkflow(ctx, 'open-url');
  assert.equal(result.status, 'completed');
  assert.deepEqual(result.target, { tabId: 9, windowId: 4 });
  assert.equal(result.steps.length, 2);
});
