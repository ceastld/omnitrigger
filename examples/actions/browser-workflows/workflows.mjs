// 此模块只组合既有 ctx.browser API；不读前台窗口、剪贴板或隐含的活动标签。
const COMMON = ['profile', 'timeoutMs'];
const OPEN = ['windowId', 'tabId', 'activate', 'focusWindow'];
/** @type {Record<string,string[]>} */
const FIELDS = {
  'create-default-tab': [...COMMON, 'windowId', 'activate', 'focusWindow'],
  'open-url': [...COMMON, ...OPEN, 'url'],
  'open-bookmark': [...COMMON, ...OPEN, 'query', 'bookmarkId'],
  'duplicate-tab': [...COMMON, 'windowId', 'tabId', 'focusWindow'],
  'restore-session': [...COMMON, 'sessionId', 'sessionKind'],
};

/** @param {string} code @param {string} message @param {string} [method] @param {string[]} [requiredPermissions] @returns {never} */
function fail(code, message, method = 'browser.workflow', requiredPermissions) {
  throw Object.assign(new Error(message), { code, method, effectState: 'not_started',
    ...(requiredPermissions ? { requiredPermissions } : {}) });
}
/** @param {unknown} value @returns {value is number} */
function id(value) { return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 && value <= 2147483647; }
/** @param {unknown} value @returns {value is string} */
function text(value, max = 128) {
  return typeof value === 'string' && value.trim().length > 0 && value.length <= max && !/[\u0000-\u001f\u007f]/u.test(value);
}
/** @typedef {new (input:string) => Readonly<{href:string,protocol:string,username:string,password:string}>} WorkflowUrlConstructor */
/** @param {unknown} value */
function canonicalUrl(value, response = false) {
  try {
    if (!text(value, 16384)) throw new Error();
    const URL = /** @type {typeof globalThis & {URL?:WorkflowUrlConstructor}} */ (globalThis).URL;
    if (typeof URL !== 'function') throw new Error();
    const url = new URL(value);
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) throw new Error();
    return url.href;
  } catch {
    fail(response ? 'unsupported_url' : 'invalid_input', '网址需要不含凭据的完整 HTTP 或 HTTPS URL。');
  }
}
/** @typedef {{profile?:string,windowId:number,tabId:number,focusWindow?:boolean,timeoutMs?:number}} DuplicateInput */
/** @typedef {{profile?:string,timeoutMs?:number,sessionId:string,sessionKind:'tab'|'window'}} RestoreInput */
/**
 * @overload
 * @param {unknown} input
 * @param {'restore-session'} operation
 * @returns {asserts input is RestoreInput}
 */
/** @overload @param {unknown} input @param {'duplicate-tab'} operation @returns {asserts input is DuplicateInput} */
/** @overload @param {unknown} input @param {string} operation @returns {void} */
/** @param {any} input @param {string} operation */
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
  if (operation === 'create-default-tab' && !text(input.profile)) {
    fail('invalid_input', '默认新标签页需要明确 profile。');
  }
  if (operation === 'open-bookmark' && (!text(input.query, 512)
      || (input.bookmarkId !== undefined && !text(input.bookmarkId)))) {
    fail('invalid_input', '书签搜索需要 query；可用 bookmarkId 消除多个匹配。');
  }
}

/** @param {unknown} profiles @param {any} input @returns {string} */
function selectedProfile(profiles, input) {
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

/** @param {import('./sdk/api').BrowserApi} browser @param {any} input */
async function selectProfile(browser, input) {
  const profiles = await browser.profiles({ timeoutMs: input.timeoutMs ?? 10000 });
  return selectedProfile(profiles, input);
}

/** @param {any} tab @param {number} [expectedWindow] @param {number} [expectedTab] */
function tabIdentity(tab, expectedWindow, expectedTab) {
  if (!tab || !id(tab.id) || !id(tab.windowId)) fail('invalid_response', '标签回执缺少明确 ID。');
  if ((expectedWindow !== undefined && tab.windowId !== expectedWindow)
      || (expectedTab !== undefined && tab.id !== expectedTab)) {
    fail('target_changed', '标签已经离开指定窗口或返回了其他标签，停止操作。');
  }
  return { tabId: tab.id, windowId: tab.windowId };
}
/** @param {any} tab */
function tabUrl(tab) {
  // 导航中的 pendingUrl 优先，避免复用正在离开目标网页的标签。
  const url = tab.pendingUrl ?? tab.url;
  if (typeof url !== 'string') {
    fail('permission_required', '无法读取窗口内全部标签网址，不能可靠判断是否需要新建。', 'tabs.query', ['tabs']);
  }
  try {
    const URL = /** @type {typeof globalThis & {URL?:WorkflowUrlConstructor}} */ (globalThis).URL;
    if (typeof URL !== 'function') throw new Error();
    return new URL(url).href;
  } catch { fail('invalid_response', '标签网址格式无效。', 'tabs.query'); }
}


/** @typedef {{status:string,steps:unknown[],error?:{code:string,method:string,effect_state:string,required_permissions?:string[]}}} WorkflowReceipt */
/** @template {WorkflowReceipt} T
 * @param {T} receipt @param {boolean} attemptedMutation @param {string} currentMethod @param {any} error @returns {T} */
function workflowFailure(receipt, attemptedMutation, currentMethod, error) {
  if (!attemptedMutation) throw error;
  const steps = receipt.steps;
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

/** @typedef {{method:string,args?:unknown[],options:{profile?:string,timeoutMs:number}}} DefaultTabRequest */
/** @typedef {{method:string,windowId?:number,tabId?:number}} DefaultTabStep */
/** @typedef {{status:string,profile:string,operation:string,steps:DefaultTabStep[],addressBarRequested?:boolean,
 * disposition?:string,target?:{tabId:number,windowId:number},error?:{code:string,method:string,effect_state:string,required_permissions?:string[]}}} DefaultTabReceipt */
// 只为默认新标签动作共享有序业务步骤；两种驱动不解释第二套参数/回执规则。
/** @param {any} rawInput @returns {Generator<DefaultTabRequest,DefaultTabReceipt,any>} */
function* defaultTabSteps(rawInput) {
  const input = Object.freeze({ ...rawInput });
  validate(input, 'create-default-tab');
  const profile = selectedProfile(yield { method:'browser.profiles', options:{timeoutMs:input.timeoutMs ?? 10000} }, input);
  /** @type {DefaultTabStep[]} */
  const steps = [];
  /** @type {DefaultTabReceipt} */
  const receipt = { status:'completed', profile, operation:'create-default-tab', steps, addressBarRequested:false };
  let attemptedMutation = false;
  let currentMethod = 'browser.workflow';
  try {
    currentMethod = 'windows.get';
    const window = yield { method:currentMethod, args:[input.windowId], options:{profile,timeoutMs:input.timeoutMs ?? 10000} };
    if (!window || window.id !== input.windowId || window.type !== 'normal') {
      fail('target_changed', '指定普通浏览器窗口不可用。', 'windows.get');
    }
    currentMethod = 'tabs.create';
    attemptedMutation = true;
    // url始终省略，由浏览器选择默认页；失败/未知均不自动重派。
    const created = yield { method:currentMethod, args:[{windowId:input.windowId,active:input.activate !== false}], options:{profile,timeoutMs:input.timeoutMs ?? 10000} };
    steps.push({method:currentMethod,windowId:input.windowId});
    const identity = tabIdentity(created);
    Object.assign(steps[steps.length - 1], identity);
    tabIdentity(created, input.windowId);
    Object.assign(receipt, {disposition:'created',target:identity});
    if (input.activate !== false && created.active !== true) {
      fail('activation_unconfirmed', '新标签回执未确认已激活。', 'tabs.create');
    }
    if (input.focusWindow === true) {
      currentMethod = 'windows.update';
      attemptedMutation = true;
      const focused = yield { method:currentMethod, args:[input.windowId,{focused:true}], options:{profile,timeoutMs:input.timeoutMs ?? 10000} };
      steps.push({method:currentMethod,windowId:input.windowId});
      if (!focused || !id(focused.id)) fail('invalid_response', '聚焦回执缺少明确窗口 ID。', 'windows.update');
      if (focused.id !== input.windowId) fail('target_changed', '聚焦回执返回了其他窗口。', 'windows.update');
      if (focused.focused !== true) fail('focus_unconfirmed', '回执未确认指定浏览器窗口已聚焦。', 'windows.update');
    }
    return receipt;
  } catch (error) {
    return workflowFailure(receipt, attemptedMutation, currentMethod, error);
  }
}
/** @param {ScriptContext['browser']|import('./sdk/api').BrowserApi} browser @param {DefaultTabRequest} request */
function invokeDefaultTab(browser, request) {
  return request.method === 'browser.profiles' ? browser.profiles(request.options)
    : browser.call(request.method, request.args, request.options);
}
/** @param {Pick<import('./sdk/api').ActionContext,'input'|'browser'>} ctx @returns {Promise<DefaultTabReceipt>} */
async function runDefaultTabAsync(ctx) {
  const flow = defaultTabSteps(ctx.input);
  let step = flow.next();
  while (!step.done) {
    let value;
    try { value = await invokeDefaultTab(ctx.browser, step.value); }
    catch (error) { step = flow.throw(error); continue; }
    step = flow.next(value);
  }
  return step.value;
}
/** @param {Pick<ScriptContext,'input'|'browser'>} ctx @returns {DefaultTabReceipt} */
export function runDefaultTabSync(ctx) {
  const flow = defaultTabSteps(ctx.input);
  let step = flow.next();
  while (!step.done) {
    let value;
    try { value = invokeDefaultTab(ctx.browser, step.value); }
    catch (error) { step = flow.throw(error); continue; }
    step = flow.next(value);
  }
  return step.value;
}

/** @typedef {{method:string,args?:unknown[],options:ScriptBrowserOptions}} RestoreRequest */
/** @typedef {{method:string,sessionId:string,sessionKind:'tab'|'window'}} RestoreStep */
/** @typedef {{status:string,profile:string,operation:string,steps:RestoreStep[],
 * target?:{windowId:number,tabId?:number},error?:{code:string,method:string,effect_state:string,required_permissions?:string[]}}} RestoreReceipt */
/** @typedef {{tab?:{sessionId?:unknown}|null,window?:{sessionId?:unknown}|null}} ClosedSession */
// 恢复只共享已有有序业务，不重选目标、不回退最近会话、不申请权限。
/** @param {unknown} rawInput @returns {Generator<RestoreRequest,RestoreReceipt,unknown>} */
function* restoreSessionSteps(rawInput) {
  const input = Object.freeze({ .../** @type {Record<string,unknown>} */ (rawInput) });
  validate(input, 'restore-session');
  const profile = selectedProfile(yield {method:'browser.profiles',options:{timeoutMs:input.timeoutMs ?? 10000}}, input);
  /** @type {RestoreStep[]} */
  const steps = [];
  /** @type {RestoreReceipt} */
  const receipt = {status:'completed',profile,operation:'restore-session',steps};
  const options = {profile,timeoutMs:input.timeoutMs ?? 10000};
  let attemptedMutation = false;
  let currentMethod = 'sessions.getRecentlyClosed';
  try {
    const sessions = yield {method:currentMethod,args:[{maxResults:25}],options:{...options}};
    if (!Array.isArray(sessions)) fail('invalid_response', '关闭会话列表格式无效。', currentMethod);
    const matches = /** @type {(ClosedSession|null|undefined)[]} */ (sessions)
      .filter(session => session?.[input.sessionKind]?.sessionId === input.sessionId);
    if (matches.length !== 1) fail('session_unavailable', '指定类型的关闭会话不存在或不唯一，未恢复其他会话。');
    currentMethod = 'sessions.restore';
    attemptedMutation = true;
    const restored = /** @type {{tab?:unknown,window?:unknown}|null|undefined} */
      (yield {method:currentMethod,args:[input.sessionId],options:{...options}});
    // API已确认；结果缺少身份也必须保留已发生的步骤。
    steps.push({method:currentMethod,sessionId:input.sessionId,sessionKind:input.sessionKind});
    const target = restored?.[input.sessionKind];
    if (input.sessionKind === 'tab') receipt.target = tabIdentity(target);
    else {
      const window = /** @type {{id?:unknown}|null|undefined} */ (target);
      if (!window || !id(window.id)) fail('invalid_response', '恢复窗口回执缺少 ID。');
      receipt.target = {windowId:window.id};
    }
    return receipt;
  } catch (error) {
    return workflowFailure(receipt, attemptedMutation, currentMethod, error);
  }
}
/** @param {ScriptContext['browser']|import('./sdk/api').BrowserApi} browser @param {RestoreRequest} request */
function invokeRestoreSession(browser, request) {
  return request.method === 'browser.profiles' ? browser.profiles(request.options)
    : browser.call(request.method, request.args, request.options);
}
/** @param {Pick<import('./sdk/api').ActionContext,'input'|'browser'>} ctx @returns {Promise<RestoreReceipt>} */
async function runRestoreSessionAsync(ctx) {
  const flow = restoreSessionSteps(ctx.input);
  let step = flow.next();
  while (!step.done) {
    let value;
    try { value = await invokeRestoreSession(ctx.browser, step.value); }
    catch (error) { step = flow.throw(error); continue; }
    step = flow.next(value);
  }
  return step.value;
}
/** @param {Pick<ScriptContext,'input'|'browser'>} ctx @returns {RestoreReceipt} */
export function runRestoreSessionSync(ctx) {
  const flow = restoreSessionSteps(ctx.input);
  let step = flow.next();
  while (!step.done) {
    let value;
    try { value = invokeRestoreSession(ctx.browser, step.value); }
    catch (error) { step = flow.throw(error); continue; }
    step = flow.next(value);
  }
  return step.value;
}

/** @typedef {{method:string,args?:unknown[],options:ScriptBrowserOptions}} DuplicateRequest */
/** @typedef {{method:string,windowId:number,sourceTabId?:number,tabId?:number}} DuplicateStep */
/** @typedef {{status:string,profile:string,operation:string,steps:DuplicateStep[],
 * target?:{tabId:number,windowId:number},error?:{code:string,method:string,effect_state:string,required_permissions?:string[]}}} DuplicateReceipt */
// 复制两种入口共用输入快照、身份核对与有序副作用；不查询活动标签或重派。
/** @param {unknown} rawInput @returns {Generator<DuplicateRequest,DuplicateReceipt,unknown>} */
function* duplicateTabSteps(rawInput) {
  const input = Object.freeze({.../** @type {Record<string,unknown>} */ (rawInput)});
  validate(input, 'duplicate-tab');
  const profile = selectedProfile(yield {method:'browser.profiles',options:{timeoutMs:input.timeoutMs ?? 10000}}, input);
  /** @type {DuplicateStep[]} */
  const steps = [];
  /** @type {DuplicateReceipt} */
  const receipt = {status:'completed',profile,operation:'duplicate-tab',steps};
  let attemptedMutation = false;
  let currentMethod = 'tabs.get';
  try {
    const source = yield {method:currentMethod,args:[input.tabId],options:{profile,timeoutMs:input.timeoutMs ?? 10000}};
    tabIdentity(source, input.windowId, input.tabId);
    currentMethod = 'tabs.duplicate';
    attemptedMutation = true;
    const copy = yield {method:currentMethod,args:[input.tabId],options:{profile,timeoutMs:input.timeoutMs ?? 10000}};
    steps.push({method:currentMethod,sourceTabId:input.tabId,windowId:input.windowId});
    const identity = tabIdentity(copy);
    Object.assign(steps[steps.length - 1], identity);
    tabIdentity(copy, input.windowId);
    if (identity.tabId === input.tabId) fail('invalid_response', '复制回执返回了源标签 ID，未确认独立副本。', 'tabs.duplicate');
    receipt.target = identity;
    if (input.focusWindow === true) {
      currentMethod = 'windows.update';
      const focused = /** @type {{id?:unknown,focused?:unknown}|null|undefined} */
        (yield {method:currentMethod,args:[identity.windowId,{focused:true}],options:{profile,timeoutMs:input.timeoutMs ?? 10000}});
      steps.push({method:currentMethod,windowId:identity.windowId});
      if (!focused || !id(focused.id)) fail('invalid_response', '聚焦回执缺少明确窗口 ID。', 'windows.update');
      if (focused.id !== identity.windowId) fail('target_changed', '聚焦回执返回了其他窗口。', 'windows.update');
      if (focused.focused !== true) fail('focus_unconfirmed', '回执未确认指定浏览器窗口已聚焦。', 'windows.update');
    }
    return receipt;
  } catch (error) {
    return workflowFailure(receipt, attemptedMutation, currentMethod, error);
  }
}
/** @param {ScriptContext['browser']|import('./sdk/api').BrowserApi} browser @param {DuplicateRequest} request */
function invokeDuplicateTab(browser, request) {
  return request.method === 'browser.profiles' ? browser.profiles(request.options)
    : browser.call(request.method, request.args, request.options);
}
/** @param {Pick<import('./sdk/api').ActionContext,'input'|'browser'>} ctx @returns {Promise<DuplicateReceipt>} */
export async function runDuplicateTab(ctx) {
  const flow = duplicateTabSteps(ctx.input);
  let step = flow.next();
  while (!step.done) {
    let value;
    try { value = await invokeDuplicateTab(ctx.browser, step.value); }
    catch (error) { step = flow.throw(error); continue; }
    step = flow.next(value);
  }
  return step.value;
}
/** @param {Pick<ScriptContext,'input'|'browser'>} ctx @returns {DuplicateReceipt} */
export function runDuplicateTabSync(ctx) {
  const flow = duplicateTabSteps(ctx.input);
  let step = flow.next();
  while (!step.done) {
    let value;
    try { value = invokeDuplicateTab(ctx.browser, step.value); }
    catch (error) { step = flow.throw(error); continue; }
    step = flow.next(value);
  }
  return step.value;
}

/** @param {import('./sdk/api').ActionContext} ctx @param {string} operation */
export async function runWorkflow(ctx, operation) {
  if (operation === 'create-default-tab') return runDefaultTabAsync(ctx);
  if (operation === 'restore-session') return runRestoreSessionAsync(ctx);
  if (operation === 'duplicate-tab') return runDuplicateTab(ctx);
  const input = /** @type {any} */ (Object.freeze({ ...ctx.input }));
  validate(input, operation);
  const profile = await selectProfile(ctx.browser, input);
  /** @type {any[]} */
  const steps = [];
  let attemptedMutation = false;
  let currentMethod = 'browser.workflow';
  /** @type {WorkflowReceipt & Record<string,any>} */
  const receipt = { status: 'completed', profile, operation, steps };
  /** @param {string} method @param {unknown[]} args @returns {Promise<any>} */
  const call = async (method, args) => {
    currentMethod = method;
    return ctx.browser.call(method, args, { profile, timeoutMs: input.timeoutMs ?? 10000 });
  };
  /** @param {string} method @param {unknown[]} args @param {Record<string,unknown>} describe */
  const mutate = async (method, args, describe) => {
    attemptedMutation = true;
    const result = await call(method, args);
    // 先记录调用确认完成；即使结果缺少 ID，也不能称作尚未执行。
    steps.push({ method, ...describe });
    return result;
  };
  /** @param {number} windowId */
  const focus = async windowId => {
    if (input.focusWindow === true) {
      const focused = await mutate('windows.update', [windowId, { focused: true }], { windowId });
      if (!focused || !id(focused.id)) fail('invalid_response', '聚焦回执缺少明确窗口 ID。', 'windows.update');
      if (focused.id !== windowId) fail('target_changed', '聚焦回执返回了其他窗口。', 'windows.update');
      if (focused.focused !== true) fail('focus_unconfirmed', '回执未确认指定浏览器窗口已聚焦。', 'windows.update');
    }
  };
  /** @param {string} url */
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
      if (input.activate !== false) {
        const activated = await mutate('tabs.update', [selected.id, { active: true }], identity);
        // Chrome 允许成功但无返回值；调用已确认，目标激活状态仍未确认。
        if (activated === undefined) fail('activation_unconfirmed', '激活调用未返回可核对的标签。', 'tabs.update');
        tabIdentity(activated, input.windowId, selected.id);
        if (activated.active !== true) fail('activation_unconfirmed', '回执未确认目标标签已激活。', 'tabs.update');
      }
    } else {
      const created = await mutate('tabs.create', [{ windowId: input.windowId, url, active: input.activate !== false }], { windowId: input.windowId });
      const identity = tabIdentity(created);
      Object.assign(steps[steps.length - 1], identity);
      tabIdentity(created, input.windowId);
      Object.assign(receipt, { disposition: 'created', target: identity });
      if (input.activate !== false && created.active !== true) {
        fail('activation_unconfirmed', '新标签回执未确认已激活。', 'tabs.create');
      }
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
    }
    return receipt;
  } catch (error) {
    return workflowFailure(receipt, attemptedMutation, currentMethod, error);
  }
}
