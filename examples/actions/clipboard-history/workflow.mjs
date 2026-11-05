// 同一份纯流程由 Node 异步入口和轻量同步入口驱动；模块本身不读取任何系统数据。
const maximumId = '18446744073709551615';
/** @param {unknown} value @returns {value is Record<string,unknown>} */
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
/** @param {unknown} value @param {readonly string[]} expected @returns {value is Record<string,unknown>} */
const keys = (value, expected) => object(value) && Reflect.ownKeys(value).length === expected.length
  && expected.every(key => Object.hasOwn(value, key));
/** @param {unknown} value @param {boolean} [zero] @returns {value is string} */
const decimal = (value, zero = false) => typeof value === 'string'
  && /^(0|[1-9][0-9]*)$/.test(value) && (zero || value !== '0')
  && (value.length < 20 || value.length === 20 && value <= maximumId);

/** @param {unknown} value @param {number} maximum @returns {value is string} */
function text(value, maximum) {
  if (typeof value !== 'string' || value.length > maximum || value.includes('\0')) return false;
  for (let index = 0; index < value.length; index++) {
    const unit = value.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(++index);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
  }
  return true;
}

/** @param {string} [code] @returns {never} */
function fail(code = 'invalid_result') {
  const beforeEffect = ['invalid_input', 'history_unavailable', 'ui_unavailable', 'native_bridge_unavailable'].includes(code);
  throw Object.assign(new Error('历史动作参数或宿主结果无效。'), {code,
    ...(beforeEffect ? {effectState:'not_started'} : {})});
}

/** @template {import('./history-types').ActionResult} Value @param {Value} value @returns {import('./history-types').Done<Value>} */
const done = value => ({done:true, value});
/** @template {import('./history-types').Method} Key @param {Key} method @param {import('./history-types').Arguments[Key]} args @param {(value:unknown)=>import('./history-types').Step} resume @param {boolean} [effectful] @returns {import('./history-types').Pending<Key>} */
const call = (method, args, resume, effectful = false) => ({done:false, method, args, resume, effectful});

/** @param {unknown} value @param {number} limit @returns {asserts value is import('./history-types').ClipboardHistoryList} */
function validateList(value, limit) {
  if (!keys(value, ['items', 'revision']) || !decimal(value.revision, true)
      || !Array.isArray(value.items) || value.items.length > limit) fail();
  /** @type {Set<string>} */
  const ids = new Set();
  for (const item of /** @type {unknown[]} */ (value.items)) {
    if (!keys(item, ['id', 'preview', 'utf16Units', 'createdAt']) || !decimal(item.id)
        || ids.has(item.id) || !text(item.preview, 64) || /[\n\r\v\f\u0085\u2028\u2029]/.test(item.preview)
        || !Number.isInteger(item.utf16Units) || /** @type {number} */ (item.utf16Units) < 0 || /** @type {number} */ (item.utf16Units) > 65536
        || !text(item.createdAt, Infinity)) fail();
    ids.add(item.id);
  }
}

/** 仅将摘要转成有效 UI 标签；从历史取出的正文始终保持原样。
 * @param {import('./history-types').ClipboardHistoryItem} item @param {number} index
 */
function label(item, index) {
  const preview = item.preview.replace(/[\x00-\x1f\x7f-\x9f\u2028\u2029]/g, ' ');
  return `${index + 1}. ${preview.trim() ? preview : '（空白文本）'}`;
}

/** @param {import('./history-types').SelectMode} mode @param {unknown} listing @param {number} limit @returns {import('./history-types').Step} */
function select(mode, listing, limit) {
  validateList(listing, limit);
  const itemCount = listing.items.length;
  if (!itemCount) return done({operation:mode, status:'empty', itemCount:0, inputSubmitted:false});
  return call('ui.choose', [{title:mode === 'send' ? '选择要发送的历史文本' : '选择历史摘要',
    items:listing.items.map((item, index) => ({id:item.id, label:label(item, index)}))}], choice => {
    if (keys(choice, ['status', 'reason']) && choice.status === 'cancelled'
        && ['dismissed', 'timeout'].includes(/** @type {string} */ (choice.reason))) {
      return done({operation:mode, status:'cancelled', itemCount, inputSubmitted:false});
    }
    if (!keys(choice, ['status', 'id']) || choice.status !== 'selected') fail();
    const selected = listing.items.find(item => item.id === choice.id);
    if (!selected) fail();
    if (mode === 'list') return done({operation:mode, status:'selected', itemCount,
      selectedUtf16Units:selected.utf16Units, inputSubmitted:false});
    return call('history.get', [{id:selected.id}], full => {
      if (!keys(full, ['id', 'text', 'createdAt', 'revision']) || full.id !== selected.id
          || !decimal(full.revision, true) || !text(full.text, 65536)
          || full.text.length !== selected.utf16Units || full.createdAt !== selected.createdAt) fail();
      if (!full.text.length) return done({operation:mode, status:'empty_text', itemCount,
        selectedUtf16Units:0, inputSubmitted:false});
      return call('keyboard.text', [full.text], submitted => {
        if (!keys(submitted, ['submitted']) || submitted.submitted !== true) fail();
        return done({operation:mode, status:'submitted', itemCount,
          selectedUtf16Units:(/** @type {string} */ (full.text)).length, inputSubmitted:true});
      }, true);
    });
  });
}

/** @param {unknown} input @returns {import('./history-types').Step} */
export function prepare(input) {
  if (!object(input)) fail('invalid_input');
  const mode = /** @type {import('./history-types').Mode} */ (input.mode);
  if (!['append', 'list', 'send'].includes(mode)) fail('invalid_input');
  if (mode === 'append') {
    if (!keys(input, ['mode', 'text'])) fail('invalid_input');
    const savedText = input.text;
    if (!text(savedText, 65536)) fail('invalid_input');
    return call('history.append', [{text:savedText}], value => {
      if (!keys(value, ['id', 'revision', 'inserted']) || !decimal(value.id)
          || !decimal(value.revision, true) || typeof value.inserted !== 'boolean') fail();
      return done({operation:'append', inserted:value.inserted, utf16Units:savedText.length});
    }, true);
  }
  if (!keys(input, ['mode']) && !keys(input, ['mode', 'limit'])) fail('invalid_input');
  const limit = /** @type {number} */ (Object.hasOwn(input, 'limit') ? input.limit : 100);
  // 最多 100 条 64 单元摘要，给 ui.choose 的 32 KiB 请求上限留出足够空间。
  if (!Number.isInteger(limit) || limit < 1 || limit > 100) fail('invalid_input');
  return call('history.list', [{limit}], value => select(mode, value, limit));
}

/** @param {import('./history-types').SharedContext} ctx @param {import('./history-types').Call} step @returns {unknown} */
export function invoke(ctx, step) {
  if (step.method.startsWith('history.')) {
    const name = /** @type {import('./history-types').HistoryName} */ (step.method.slice('history.'.length));
    const history = ctx.clipboard?.history;
    if (!history || typeof history[name] !== 'function') fail('history_unavailable');
    return /** @type {(...args:import('./history-types').HistoryArguments)=>unknown} */ (history[name])(
      .../** @type {import('./history-types').HistoryArguments} */ (step.args));
  }
  if (step.method === 'ui.choose') {
    if (typeof ctx.ui?.choose !== 'function') fail('ui_unavailable');
    return ctx.ui.choose(...step.args);
  }
  if (typeof ctx.desktop?.sendText !== 'function') fail('native_bridge_unavailable');
  return ctx.desktop.sendText(.../** @type {[string]} */ (step.args));
}

const safeCodes = new Set(['invalid_input', 'invalid_result', 'invalid_response', 'history_unavailable', 'history_item_missing',
  'history_error', 'history_busy', 'history_conflict', 'history_corrupt', 'history_id_exhausted',
  'history_revision_exhausted', 'history_too_large', 'history_invalid_data', 'ui_unavailable', 'ui_busy',
  'ui_error', 'target_changed', 'target_unavailable', 'timeout', 'cancelled', 'input_failed',
  'native_bridge_unavailable', 'access_denied', 'history_failed']);

/** @param {unknown} error @param {import('./history-types').ActionMethod} method @param {boolean} [effectful] @returns {import('./history-types').HistoryActionFault} */
export function redact(error, method, effectful = false) {
  let code = 'history_action_failed';
  /** @type {import('./history-types').EffectState} */
  let effectState = effectful ? 'unknown' : 'not_started';
  try {
    const reported = (/** @type {import('./history-types').FaultInfo|null|undefined} */ (error))?.code;
    if (safeCodes.has(/** @type {string} */ (reported))) code = /** @type {string} */ (reported);
    const state = (/** @type {import('./history-types').FaultInfo|null|undefined} */ (error))?.effectState;
    if (['not_started', 'unknown'].includes(/** @type {string} */ (state))) effectState = /** @type {import('./history-types').EffectState} */ (state);
  } catch (_) { /* 异常 getter 也不能让宿主正文逸出。 */ }
  return Object.assign(new Error('历史动作未完成，请查看错误代码。'), {code, method, effectState});
}
