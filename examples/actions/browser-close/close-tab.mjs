// 明确 ID 的独立关闭业务，不按标题、活动标签或前台推断目标。
const fields = ['profile', 'windowId', 'tabId', 'timeoutMs'];
/** @param {any} value */
const isId = value => Number.isSafeInteger(value) && value >= 0;
/** @param {unknown} value @returns {value is string} */
const isText = value => typeof value === 'string' && value.trim().length > 0
  && value.length <= 128 && !/[\u0000-\u001f\u007f]/u.test(value);
/** @param {string} code @param {string} message @param {string} method @returns {never} */
function fail(code, message, method) {
  throw Object.assign(new Error(message), { code, method, effectState: 'not_started' });
}

/** @typedef {{status:string,operation:string,profile:string,target:{tabId:number,windowId:number},
 * steps:{method:string,tabId:number,windowId:number}[],closeAcknowledged:boolean,
 * error?:{code:string,method:string,effect_state:string,required_permissions?:string[]}}} CloseReceipt */
/** @param {import('./sdk/api').ActionContext} ctx @returns {Promise<CloseReceipt>} */
export default async function closeTab(ctx) {
  const raw = ctx.input;
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)
      || Object.keys(raw).some(key => !fields.includes(key))) {
    fail('invalid_input', '关闭参数需要只包含已知字段的对象。', 'browser.closeTab');
  }
  const input = Object.freeze({ ...raw });
  if (!isText(input.profile) || !isId(input.windowId) || !isId(input.tabId)
      || (input.timeoutMs !== undefined && (!Number.isInteger(input.timeoutMs)
        || input.timeoutMs < 1 || input.timeoutMs > 60000))) {
    fail('invalid_input', '关闭需要明确 profile、windowId、tabId 和有效期限。', 'browser.closeTab');
  }
  const options = Object.freeze({ profile: input.profile, timeoutMs: input.timeoutMs ?? 10000 });
  const profiles = await ctx.browser.profiles({ timeoutMs: options.timeoutMs });
  if (!Array.isArray(profiles) || profiles.some(p => !p || !isText(p.browser_instance_id))
      || new Set(profiles.map(p => p.browser_instance_id)).size !== profiles.length) {
    fail('invalid_response', '配置列表缺少唯一有效身份。', 'browser.profiles');
  }
  if (!profiles.some(p => p.browser_instance_id === input.profile)) {
    fail('profile_unavailable', '指定浏览器配置没有连接。', 'browser.profiles');
  }
  const window = /** @type {any} */ (await ctx.browser.call('windows.get', [input.windowId], options));
  if (!window || window.id !== input.windowId || !['normal', 'popup'].includes(window.type)) {
    fail('target_changed', '指定浏览器窗口不可用。', 'windows.get');
  }
  const tab = /** @type {any} */ (await ctx.browser.call('tabs.get', [input.tabId], options));
  if (!tab || tab.id !== input.tabId || tab.windowId !== input.windowId) {
    fail('target_changed', '指定标签已消失或离开目标窗口。', 'tabs.get');
  }
  const target = { tabId: input.tabId, windowId: input.windowId };
  /** @type {CloseReceipt} */
  const receipt = { status: 'completed', operation: 'close-tab', profile: input.profile,
    target, steps: [], closeAcknowledged: false };
  try {
    // 唯一副作用。SDK 成功响应只用于确认此次 remove 调用，不读取活动标签猜结果。
    await ctx.browser.call('tabs.remove', [input.tabId], options);
    receipt.steps.push({ method: 'tabs.remove', ...target });
    receipt.closeAcknowledged = true;
    return receipt;
  } catch (caught) {
    const error = /** @type {any} */ (caught);
    const effect = ['not_started', 'partial', 'unknown'].includes(error?.effectState)
      ? error.effectState : 'unknown';
    receipt.status = effect === 'not_started' ? 'failed' : effect === 'partial' ? 'partial' : 'unknown';
    receipt.error = {
      code: typeof error?.code === 'string' ? error.code : 'browser_workflow_failed',
      method: 'tabs.remove', effect_state: effect,
      ...(Array.isArray(error?.requiredPermissions)
        ? { required_permissions: error.requiredPermissions.filter(/** @param {unknown} value */ value => typeof value === 'string') } : {}),
    };
    return receipt;
  }
}
