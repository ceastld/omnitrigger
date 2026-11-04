// 此模块只组合既有 ctx.browser API；不读前台窗口、剪贴板或隐含的活动标签。
const COMMON = ['profile', 'timeoutMs'];
const OPEN = ['windowId', 'tabId', 'activate', 'focusWindow'];
const FIELDS = {
  'open-url': [...COMMON, ...OPEN, 'url'],
  'open-bookmark': [...COMMON, ...OPEN, 'query', 'bookmarkId'],
  'duplicate-tab': [...COMMON, 'windowId', 'tabId', 'focusWindow'],
  'restore-session': [...COMMON, 'sessionId', 'sessionKind'],
};

function fail(code, message, method = 'browser.workflow', requiredPermissions) {
  throw Object.assign(new Error(message), { code, method, effectState: 'not_started',
    ...(requiredPermissions ? { requiredPermissions } : {}) });
}
function id(value) { return Number.isSafeInteger(value) && value >= 0; }
function text(value, max = 128) {
  return typeof value === 'string' && value.trim().length > 0 && value.length <= max && !/[\u0000-\u001f\u007f]/u.test(value);
}
function canonicalUrl(value, response = false) {
  try {
    if (!text(value, 16384)) throw new Error();
    const url = new URL(value);
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) throw new Error();
    return url.href;
  } catch {
    fail(response ? 'unsupported_url' : 'invalid_input', '网址需要不含凭据的完整 HTTP 或 HTTPS URL。');
  }
}
function validate(input, operation) {
  if (!FIELDS[operation] || !input || typeof input !== 'object' || Array.isArray(input)
      || Object.keys(input).some(key => !FIELDS[operation].includes(key))) {
    fail('invalid_input', '工作流参数包含未知字段或不是对象。');
  }
  if ((input.profile !== undefined && !text(input.profile))
      || (input.timeoutMs !== undefined && (!Number.isInteger(input.timeoutMs) || input.timeoutMs < 1 || input.timeoutMs > 60000))) {
    fail('invalid_input', 'profile 或 timeoutMs 无效。');
  }
  if (operation === 'restore-session') {
    if (!text(input.sessionId, 256) || !['tab', 'window'].includes(input.sessionKind)) {
      fail('invalid_input', '恢复需要明确 sessionId 以及 tab 或 window 的 sessionKind。');
    }
    return;
  }
  if (!id(input.windowId) || (input.tabId !== undefined && !id(input.tabId))
      || ['activate', 'focusWindow'].some(key => input[key] !== undefined && typeof input[key] !== 'boolean')
      || (input.focusWindow === true && input.activate === false)) {
    fail('invalid_input', '需要有效 windowId；tabId、activate 或 focusWindow 无效。');
  }
  if (operation === 'duplicate-tab' && !id(input.tabId)) fail('invalid_input', '复制需要明确 tabId。');
  if (operation === 'open-url') canonicalUrl(input.url);
  if (operation === 'open-bookmark' && (!text(input.query, 512)
      || (input.bookmarkId !== undefined && !text(input.bookmarkId)))) {
    fail('invalid_input', '书签搜索需要 query；可用 bookmarkId 消除多个匹配。');
  }
}

async function selectProfile(browser, input) {
  const profiles = await browser.profiles({ timeoutMs: input.timeoutMs ?? 10000 });
  if (!Array.isArray(profiles) || profiles.some(p => !p || !text(p.browser_instance_id))
      || new Set(profiles.map(p => p.browser_instance_id)).size !== profiles.length) {
    fail('invalid_response', '浏览器配置列表缺少唯一有效标识。', 'browser.profiles');
  }
  if (input.profile !== undefined) {
    if (!profiles.some(p => p.browser_instance_id === input.profile)) fail('profile_unavailable', '指定浏览器配置没有连接。');
    return input.profile;
  }
  if (!profiles.length) fail('browser_unavailable', '没有已连接的浏览器配置。');
  if (profiles.length !== 1) fail('ambiguous_profile', '有多个浏览器配置，请明确填写 profile。');
  return profiles[0].browser_instance_id;
}

function tabIdentity(tab, expectedWindow, expectedTab) {
  if (!tab || !id(tab.id) || !id(tab.windowId)) fail('invalid_response', '标签回执缺少明确 ID。');
  if ((expectedWindow !== undefined && tab.windowId !== expectedWindow)
      || (expectedTab !== undefined && tab.id !== expectedTab)) {
    fail('target_changed', '标签已经离开指定窗口或返回了其他标签，停止操作。');
  }
  return { tabId: tab.id, windowId: tab.windowId };
}
function tabUrl(tab) {
  // 导航中的 pendingUrl 优先，避免复用正在离开目标网页的标签。
  const url = tab.pendingUrl ?? tab.url;
  if (typeof url !== 'string') {
    fail('permission_required', '无法读取窗口内全部标签网址，不能可靠判断是否需要新建。', 'tabs.query', ['tabs']);
  }
  try { return new URL(url).href; } catch { fail('invalid_response', '标签网址格式无效。', 'tabs.query'); }
}

/** @param {import('../../../crates/omnitrigger-actions/runtime/api').ActionContext} ctx */
export async function runWorkflow(ctx, operation) {
  const input = ctx.input;
  validate(input, operation);
  const profile = await selectProfile(ctx.browser, input);
  const steps = [];
  let attemptedMutation = false;
  let currentMethod = 'browser.workflow';
  const receipt = { status: 'completed', profile, operation, steps };
  const call = async (method, args) => {
    currentMethod = method;
    return ctx.browser.call(method, args, { profile, timeoutMs: input.timeoutMs ?? 10000 });
  };
  const mutate = async (method, args, describe) => {
    attemptedMutation = true;
    const result = await call(method, args);
    // 先记录调用确认完成；即使结果缺少 ID，也不能称作尚未执行。
    steps.push({ method, ...describe });
    return result;
  };
  const focus = async windowId => {
    if (input.focusWindow === true) await mutate('windows.update', [windowId, { focused: true }], { windowId });
  };
  const open = async url => {
    const window = await call('windows.get', [input.windowId]);
    if (!window || window.id !== input.windowId || window.type !== 'normal') {
      fail('target_changed', '指定普通浏览器窗口不可用。', 'windows.get');
    }
    const tabs = await call('tabs.query', [{ windowId: input.windowId }]);
    if (!Array.isArray(tabs)) fail('invalid_response', '标签列表格式无效。', 'tabs.query');
    const ids = new Set();
    const matches = tabs.filter(tab => {
      tabIdentity(tab, input.windowId);
      if (ids.has(tab.id)) fail('invalid_response', '标签列表包含重复 ID。', 'tabs.query');
      ids.add(tab.id);
      return tabUrl(tab) === url;
    });
    let selected;
    if (input.tabId !== undefined) {
      selected = matches.find(tab => tab.id === input.tabId);
      if (!selected) fail('target_changed', '指定标签不在目标窗口或网址不匹配，未新建替代标签。');
    } else {
      if (matches.length > 1) fail('ambiguous_tab', '同一窗口有多个相同网址，请明确填写 tabId。');
      selected = matches[0];
    }
    if (selected) {
      // 复查明确 ID；不查询 lastFocusedWindow 或 currentWindow。
      const latest = await call('tabs.get', [selected.id]);
      const identity = tabIdentity(latest, input.windowId, selected.id);
      if (tabUrl(latest) !== url) fail('target_changed', '目标标签在查询后发生导航，停止操作。');
      Object.assign(receipt, { disposition: 'reused', target: identity });
      if (input.activate !== false) await mutate('tabs.update', [selected.id, { active: true }], identity);
    } else {
      const created = await mutate('tabs.create', [{ windowId: input.windowId, url, active: input.activate !== false }], { windowId: input.windowId });
      const identity = tabIdentity(created);
      Object.assign(steps[steps.length - 1], identity);
      tabIdentity(created, input.windowId);
      Object.assign(receipt, { disposition: 'created', target: identity });
    }
    await focus(input.windowId);
  };
  try {
    if (operation === 'open-url') {
      await open(canonicalUrl(input.url));
    } else if (operation === 'open-bookmark') {
      const results = await call('bookmarks.search', [{ query: input.query }]);
      if (!Array.isArray(results) || results.some(node => !node || !text(node.id))) {
        fail('invalid_response', '书签搜索结果格式无效。', 'bookmarks.search');
      }
      const matches = results.filter(node => typeof node.url === 'string'
        && (input.bookmarkId === undefined || node.id === input.bookmarkId));
      if (!matches.length) fail('bookmark_not_found', '没有匹配的网页书签。');
      if (matches.length > 1) fail('ambiguous_bookmark', '有多个匹配书签，请明确填写 bookmarkId。');
      receipt.bookmarkId = matches[0].id;
      await open(canonicalUrl(matches[0].url, true));
    } else if (operation === 'duplicate-tab') {
      tabIdentity(await call('tabs.get', [input.tabId]), input.windowId, input.tabId);
      const copy = await mutate('tabs.duplicate', [input.tabId], { sourceTabId: input.tabId, windowId: input.windowId });
      const identity = tabIdentity(copy);
      Object.assign(steps[steps.length - 1], identity);
      tabIdentity(copy, input.windowId);
      receipt.target = identity;
      await focus(identity.windowId);
    } else {
      const sessions = await call('sessions.getRecentlyClosed', [{ maxResults: 25 }]);
      if (!Array.isArray(sessions)) fail('invalid_response', '关闭会话列表格式无效。', 'sessions.getRecentlyClosed');
      const matches = sessions.filter(session => session?.[input.sessionKind]?.sessionId === input.sessionId);
      if (matches.length !== 1) fail('session_unavailable', '指定类型的关闭会话不存在或不唯一，未恢复其他会话。');
      const restored = await mutate('sessions.restore', [input.sessionId], { sessionId: input.sessionId, sessionKind: input.sessionKind });
      const target = restored?.[input.sessionKind];
      if (input.sessionKind === 'tab') receipt.target = tabIdentity(target);
      else {
        if (!target || !id(target.id)) fail('invalid_response', '恢复窗口回执缺少 ID。');
        receipt.target = { windowId: target.id };
      }
    }
    return receipt;
  } catch (error) {
    if (!attemptedMutation) throw error;
    // 宿主异常仅保留固定字段；用输出回执保存已经确认的步骤与目标身份。
    // 未确认副作用绝不重试；不以补偿关闭标签掩盖部分成功。
    const effect = error?.effectState ?? 'unknown';
    receipt.status = steps.length ? 'partial' : (effect === 'not_started' ? 'failed' : 'unknown');
    receipt.error = {
      code: typeof error?.code === 'string' ? error.code : 'browser_workflow_failed',
      method: currentMethod,
      effect_state: steps.length && effect === 'not_started' ? 'partial' : effect,
      ...(Array.isArray(error?.requiredPermissions) ? { required_permissions: error.requiredPermissions } : {}),
    };
    return receipt;
  }
}
