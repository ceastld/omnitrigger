// 只接收明确浏览器身份，不使用活动标签、原生前台或隐含 ID=0。
const fields = ['profile', 'windowId', 'tabId', 'focusWindow', 'timeoutMs'];
/** @param {unknown} value @returns {value is number} */
const isId = value => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 && value <= 2147483647;
/** @param {unknown} value @returns {value is string} */
const isText = value => typeof value === 'string' && value.trim().length > 0
  && value.length <= 128 && !/[\u0000-\u001f\u007f]/u.test(value);
/** @param {string} code @param {string} message @param {string} method @returns {never} */
function fail(code, message, method) {
  throw Object.assign(new Error(message), { code, method, effectState: 'not_started' });
}

/** @typedef {{id?:unknown,windowId?:unknown,type?:unknown,incognito?:unknown}} IdentityReply */
/** @typedef {{method:string,tabId:number,windowId?:number}} PopoutStep */
/** @typedef {{status:string,operation:string,profile:string,source:{windowId:number,tabId:number},
 * focusRequested:boolean,steps:PopoutStep[],createdWindowId?:number,disposition?:string,
 * target?:{tabId:number,windowId:number},error?:{code:string,method:string,effect_state:string,required_permissions?:string[]}}} PopoutReceipt */
/** @typedef {{code?:unknown,effectState?:unknown,requiredPermissions?:unknown}} WorkflowFault */

/** @type {ScriptMain} */
export function main(ctx) {
  const raw = /** @type {unknown} */ (ctx.input);
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)
      || Object.keys(raw).some(key => !fields.includes(key))) {
    fail('invalid_input', '参数必须是只包含已知字段的对象。', 'browser.popOutTab');
  }
  // 所有输入都是标量；首次同步调用前固定目标、聚焦意图和期限。
  const input = Object.freeze({ .../** @type {Record<string,unknown>} */ (raw) });
  if (!isText(input.profile) || !isId(input.windowId) || !isId(input.tabId)
      || (input.focusWindow !== undefined && typeof input.focusWindow !== 'boolean')
      || (input.timeoutMs !== undefined && (typeof input.timeoutMs !== 'number' || !Number.isInteger(input.timeoutMs)
        || input.timeoutMs < 1 || input.timeoutMs > 60000))) {
    fail('invalid_input', '需要明确 profile、windowId 和 tabId；聚焦选项或期限无效。', 'browser.popOutTab');
  }
  const timeoutMs = input.timeoutMs ?? 10000;
  const profiles = ctx.browser.profiles({ timeoutMs });
  if (!Array.isArray(profiles) || profiles.some(p => !p || !isText(p.browser_instance_id))
      || new Set(profiles.map(p => p.browser_instance_id)).size !== profiles.length) {
    fail('invalid_response', '配置列表缺少唯一有效身份。', 'browser.profiles');
  }
  if (!profiles.some(p => p.browser_instance_id === input.profile)) {
    fail('profile_unavailable', '指定浏览器配置没有连接。', 'browser.profiles');
  }
  const options = Object.freeze({ profile: input.profile, timeoutMs });
  let currentMethod = 'windows.get';
  /** @param {string} method @param {unknown[]} args @returns {IdentityReply|null|undefined} */
  const call = (method, args) => {
    currentMethod = method;
    return /** @type {IdentityReply|null|undefined} */ (ctx.browser.call(method, args, options));
  };
  const sourceWindow = call('windows.get', [input.windowId]);
  if (!sourceWindow || sourceWindow.id !== input.windowId || sourceWindow.type !== 'normal') {
    fail('target_changed', '指定普通源窗口不可用。', currentMethod);
  }
  if (typeof sourceWindow.incognito !== 'boolean') {
    fail('invalid_response', '源窗口缺少隐私模式身份。', currentMethod);
  }
  const incognito = sourceWindow.incognito;
  const sourceTab = call('tabs.get', [input.tabId]);
  if (!sourceTab || sourceTab.id !== input.tabId || sourceTab.windowId !== input.windowId) {
    fail('target_changed', '指定标签已经消失或离开源窗口。', currentMethod);
  }

  if (sourceTab.incognito !== incognito) {
    fail('target_changed', '源标签与窗口的隐私上下文不符。', currentMethod);
  }

  /** @type {PopoutReceipt} */
  const receipt = {
    status: 'completed', operation: 'pop-out-tab', profile: input.profile,
    source: { windowId: input.windowId, tabId: input.tabId },
    focusRequested: input.focusWindow === true, steps: [],
  };
  try {
    // 唯一副作用：保留同一个 tabId，不传 URL、不复制、失败不重派。
    const created = call('windows.create', [{
      tabId: input.tabId, type: 'popup', focused: input.focusWindow === true, incognito,
    }]);
    /** @type {PopoutStep} */
    const step = { method: 'windows.create', tabId: input.tabId };
    receipt.steps.push(step); // API 已确认；即使身份缺失也不能称为没有发生效果。
    if (created && isId(created.id) && created.id !== input.windowId) {
      step.windowId = created.id;
      receipt.createdWindowId = created.id;
    }
    if (!created || receipt.createdWindowId === undefined || created.type !== 'popup'
        || created.incognito !== incognito) {
      fail('invalid_response', '创建回执缺少可信的新弹出窗口身份。', currentMethod);
    }
    // create 回执的 tabs 可省略；只按原 tabId 复查，不枚举其他标签或读取网址。
    const moved = call('tabs.get', [input.tabId]);
    if (!moved || moved.id !== input.tabId || moved.windowId !== receipt.createdWindowId) {
      fail('target_changed', '未确认原标签仍在新弹出窗口。', currentMethod);
    }
    if (moved.incognito !== incognito) {
      fail('target_changed', '移动后标签的隐私上下文不符。', currentMethod);
    }
    receipt.disposition = 'moved';
    receipt.target = { tabId: input.tabId, windowId: receipt.createdWindowId };
    return receipt;
  } catch (caught) {
    const error = /** @type {WorkflowFault|null|undefined} */ (caught);
    // 已确认窗口创建后，复查失败仍保留 partial；不补偿关闭、不移动回原窗口。
    const confirmed = receipt.steps.length > 0;
    const effect = typeof error?.effectState === 'string' && ['not_started', 'partial', 'unknown'].includes(error.effectState)
      ? error.effectState : 'unknown';
    receipt.status = confirmed || effect === 'partial' ? 'partial' : effect === 'not_started' ? 'failed' : 'unknown';
    receipt.error = {
      code: typeof error?.code === 'string' ? error.code : 'browser_workflow_failed',
      method: currentMethod,
      effect_state: confirmed && effect === 'not_started' ? 'partial' : effect,
      ...(Array.isArray(error?.requiredPermissions)
        ? { required_permissions: error.requiredPermissions.filter(/** @param {unknown} value */ value => typeof value === 'string') } : {}),
    };
    return receipt;
  }
}
