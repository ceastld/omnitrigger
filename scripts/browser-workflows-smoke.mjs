// 真实动作包经应用入口调用，仅操作调用方新建的隔离 Chrome profile。
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { cp, writeFile } from 'node:fs/promises';
import { join, relative } from 'node:path';
import { spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

export async function verifyBrowserWorkflows({ call, base, scratch, app, root, profile }) {
  await cp(join(root, 'examples/actions/browser-workflows'), join(scratch, 'actions/browser-workflows'), { recursive: true });
  const marker = `workflow-${randomUUID()}`;
  const url = `${base}/${marker}/open`;
  const window = await call('windows.create', [{ url: `${base}/${marker}/start`, focused: false }]);
  const windowId = window.id;
  assert(Number.isInteger(windowId));
  const checks = [];
  let bookmark;
  const invoke = async (action, input) => {
    const path = join(scratch, `workflow-call-${randomUUID()}.json`);
    await writeFile(path, JSON.stringify({ package_id: 'examples.browser-workflows', action_id: action, action_version: 1, input: { profile, timeoutMs: 10000, ...input } }));
    const child = spawn(app, ['--config', relative(process.cwd(), join(scratch, 'bindings.json')), '--run-action', path], {
      windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = ''; let stderr = '';
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    const timer = setTimeout(() => child.kill(), 20000);
    try {
      const code = await new Promise((resolve, reject) => { child.once('error', reject); child.once('close', resolve); });
      const result = JSON.parse(stdout.trim() || stderr.trim());
      assert.equal(code, result.status === 'succeeded' ? 0 : 1, stderr);
      return result;
    } finally { clearTimeout(timer); }
  };
  const completed = async (action, input) => {
    const result = await invoke(action, input);
    assert.equal(result.status, 'succeeded', JSON.stringify(result));
    assert.equal(result.output.status, 'completed', JSON.stringify(result.output));
    assert.equal(result.output.profile, profile);
    return result.output;
  };
  const tabs = () => call('tabs.query', [{ windowId }]);
  const expectTab = async (tabId, expectedUrl) => {
    for (let attempt = 0; attempt < 100; attempt++) {
      const tab = await call('tabs.get', [tabId]);
      assert.equal(tab.windowId, windowId);
      if (tab.url === expectedUrl && !tab.pendingUrl) return tab;
      await delay(100);
    }
    assert.fail('本地测试标签导航未完成');
  };
  try {
    const before = (await tabs()).length;
    const created = await completed('open-url', { windowId, url, activate: false });
    assert.equal(created.disposition, 'created');
    await expectTab(created.target.tabId, url);
    assert.equal((await tabs()).length, before + 1);
    const reused = await completed('open-url', { windowId, url, activate: false });
    assert.equal(reused.disposition, 'reused');
    assert.deepEqual(reused.target, created.target);
    assert.equal((await tabs()).length, before + 1);
    checks.push('网址新建后复查 URL；再次运行复用同一 ID 且数量不变');

    const duplicate = await completed('duplicate-tab', { windowId, tabId: created.target.tabId });
    assert.notEqual(duplicate.target.tabId, created.target.tabId);
    await expectTab(duplicate.target.tabId, url);
    assert.equal((await tabs()).length, before + 2);
    const ambiguous = await invoke('open-url', { windowId, url, activate: false });
    assert.equal(ambiguous.status, 'failed', JSON.stringify(ambiguous));
    assert.equal(ambiguous.error.code, 'ambiguous_tab');
    assert.equal((await tabs()).length, before + 2);
    const explicit = await completed('open-url', { windowId, tabId: created.target.tabId, url, activate: false });
    assert.deepEqual(explicit.target, created.target);
    checks.push('复制产生独立 ID；同网址歧义无副作用，明确 ID 可复用');
    const wrongWindow = await invoke('duplicate-tab', { windowId: 2147483647, tabId: created.target.tabId });
    assert.equal(wrongWindow.status, 'failed');
    assert.equal(wrongWindow.error.code, 'target_changed');
    const staleProfile = await invoke('open-url', { profile: `missing-${marker}`, windowId, url, activate: false });
    assert.equal(staleProfile.status, 'failed');
    assert.equal(staleProfile.error.code, 'profile_unavailable');
    assert.equal((await tabs()).length, before + 2);
    checks.push('窗口身份不符和失联 profile 都在变更前拒绝');

    const restoreUrl = `${base}/${marker}/restore`;
    await call('tabs.update', [duplicate.target.tabId, { url: restoreUrl }]);
    await expectTab(duplicate.target.tabId, restoreUrl);
    await call('tabs.remove', [duplicate.target.tabId]);
    let session;
    for (let attempt = 0; attempt < 100; attempt++) {
      session = (await call('sessions.getRecentlyClosed', [{ maxResults: 25 }])).find(item => item.tab?.url === restoreUrl);
      if (session) break;
      await delay(100);
    }
    assert(session?.tab?.sessionId, '没有本轮关闭标签的唯一会话');
    const restored = await completed('restore-session', { sessionId: session.tab.sessionId, sessionKind: 'tab' });
    await expectTab(restored.target.tabId, restoreUrl);
    assert.equal((await tabs()).length, before + 2);
    checks.push('按本轮关闭标签的明确 sessionId 恢复并核验 URL/窗口/数量');

    const bookmarkUrl = `${base}/${marker}/bookmark`;
    bookmark = await call('bookmarks.create', [{ title: marker, url: bookmarkUrl }]);
    const opened = await completed('open-bookmark', { windowId, query: marker, bookmarkId: bookmark.id, activate: false });
    assert.equal(opened.bookmarkId, bookmark.id);
    await expectTab(opened.target.tabId, bookmarkUrl);
    assert.equal((await tabs()).length, before + 3);
    checks.push('搜索本轮书签并按明确 bookmarkId 打开，核验最终标签');
    return { ok: true, checks, windowId };
  } finally {
    try {
      if (bookmark) await call('bookmarks.remove', [bookmark.id]);
    } finally { await call('windows.remove', [windowId]); }
  }
}
