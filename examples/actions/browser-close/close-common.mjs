// 两个关闭入口只共享参数、身份与回执；业务API调用由各入口直接写出。
const fields = ['profile', 'windowId', 'tabId', 'timeoutMs'];
/** @param {unknown} value @returns {value is number} */
const isId = value => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 && value <= 2147483647;
/** @param {unknown} value @returns {value is string} */
const isText = value => typeof value === 'string' && value.trim().length > 0
  && value.length <= 128 && !/[\u0000-\u001f\u007f]/u.test(value);
/** @param {string} code @param {string} message @param {string} method @returns {never} */
function fail(code, message, method) {
  throw Object.assign(new Error(message), {code, method, effectState:'not_started'});
}
/** @typedef {{profile:string,windowId:number,tabId:number,timeoutMs:number}} CloseInput */
/** @typedef {{status:string,operation:string,profile:string,target:{tabId:number,windowId:number},
 * steps:{method:string,tabId:number,windowId:number}[],closeAcknowledged:boolean,
 * error?:{code:string,method:string,effect_state:string,required_permissions?:string[]}}} CloseReceipt */

/** @param {unknown} raw @returns {Readonly<CloseInput>} */
export function prepareInput(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)
      || Object.keys(raw).some(key => !fields.includes(key))) {
    fail('invalid_input', '关闭参数需要只包含已知字段的对象。', 'browser.closeTab');
  }
  const input = Object.freeze({...raw});
  const value = /** @type {Record<string,any>} */ (input);
  if (!isText(value.profile) || !isId(value.windowId) || !isId(value.tabId)
      || (value.timeoutMs !== undefined && (!Number.isInteger(value.timeoutMs)
        || value.timeoutMs < 1 || value.timeoutMs > 60000))) {
    fail('invalid_input', '关闭需要明确 profile、windowId、tabId 和有效期限。', 'browser.closeTab');
  }
  return Object.freeze({profile:value.profile,windowId:value.windowId,tabId:value.tabId,timeoutMs:value.timeoutMs ?? 10000});
}
/** @param {unknown} profiles @param {CloseInput} input */
export function verifyProfiles(profiles, input) {
  if (!Array.isArray(profiles) || profiles.some(p => !p || !isText(p.browser_instance_id))
      || new Set(profiles.map(p => p.browser_instance_id)).size !== profiles.length) {
    fail('invalid_response', '配置列表缺少唯一有效身份。', 'browser.profiles');
  }
  if (!profiles.some(p => p.browser_instance_id === input.profile)) {
    fail('profile_unavailable', '指定浏览器配置没有连接。', 'browser.profiles');
  }
}
/** @param {unknown} reply @param {CloseInput} input */
export function verifyWindow(reply, input) {
  const window = /** @type {any} */ (reply);
  if (!window || window.id !== input.windowId || !['normal','popup'].includes(window.type)) {
    fail('target_changed', '指定浏览器窗口不可用。', 'windows.get');
  }
}
/** @param {unknown} reply @param {CloseInput} input */
export function verifyTab(reply, input) {
  const tab = /** @type {any} */ (reply);
  if (!tab || tab.id !== input.tabId || tab.windowId !== input.windowId) {
    fail('target_changed', '指定标签已消失或离开目标窗口。', 'tabs.get');
  }
}
/** @param {CloseInput} input @returns {CloseReceipt} */
export function createReceipt(input) {
  return {status:'completed',operation:'close-tab',profile:input.profile,
    target:{tabId:input.tabId,windowId:input.windowId},steps:[],closeAcknowledged:false};
}
/** @param {CloseReceipt} receipt @returns {CloseReceipt} */
export function acknowledge(receipt) {
  receipt.steps.push({method:'tabs.remove',...receipt.target});
  receipt.closeAcknowledged = true;
  return receipt;
}
/** @param {CloseReceipt} receipt @param {unknown} caught @returns {CloseReceipt} */
export function failedReceipt(receipt, caught) {
  const error = /** @type {any} */ (caught);
  const effect = ['not_started','partial','unknown'].includes(error?.effectState) ? error.effectState : 'unknown';
  receipt.status = effect === 'not_started' ? 'failed' : effect === 'partial' ? 'partial' : 'unknown';
  receipt.error = {
    code:typeof error?.code === 'string' ? error.code : 'browser_workflow_failed',
    method:'tabs.remove',effect_state:effect,
    ...(Array.isArray(error?.requiredPermissions)
      ? {required_permissions:error.requiredPermissions.filter(/** @param {unknown} value */ value => typeof value === 'string')} : {}),
  };
  return receipt;
}
