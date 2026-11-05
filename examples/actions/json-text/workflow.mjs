import {analyze, format} from './json.mjs';

/** @param {unknown} value @returns {value is Record<string,unknown>} */
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
/** @param {unknown} value @param {readonly string[]} names @returns {value is Record<string,unknown>} */
const fields = (value, names) => object(value) && Reflect.ownKeys(value).length === names.length
  && names.every(name => Object.hasOwn(value, name));
/** @param {import('./json-types').JsonErrorCode} code @returns {never} */
const fail = code => { throw Object.assign(new Error('JSON 配方参数或结果无效。'), {code}); };

/** @param {unknown} input @returns {import('./json-types').JsonPrepared} */
export function prepare(input) {
  if (!object(input) || !['inspect','send','choose-send'].includes(/** @type {string} */ (input.mode))) fail('invalid_input');
  const mode = /** @type {import('./json-types').JsonMode} */ (input.mode);
  if (!fields(input, mode === 'send' ? ['mode','text','format'] : ['mode','text'])) fail('invalid_input');
  const result = analyze(/** @type {string} */ (input.text));
  if (mode === 'inspect') return {done:true,value:{status:'inspected',...result.summary,inputSubmitted:false}};
  const outputs = mode === 'send' ? {[/** @type {import('./json-types').Format} */ (input.format)]:format(result.tokens, input.format)}
    : {compact:format(result.tokens,'compact'),pretty:format(result.tokens,'pretty')};
  return {done:false,mode,outputs,summary:result.summary,format:mode === 'send' ? /** @type {import('./json-types').Format} */ (input.format) : undefined};
}

/** @param {import('./json-types').SharedContext} ctx */
export function choose(ctx) {
  if (typeof ctx.ui?.choose !== 'function') fail('ui_unavailable');
  return ctx.ui.choose({title:'选择 JSON 排版',items:[{id:'pretty',label:'缩进排版'},{id:'compact',label:'紧凑排版'}]});
}

/** @param {unknown} choice @returns {import('./json-types').Format|null} */
export function selection(choice) {
  if (fields(choice,['status','reason']) && choice.status === 'cancelled'
      && ['dismissed','timeout'].includes(/** @type {string} */ (choice.reason))) return null;
  if (!fields(choice,['status','id']) || choice.status !== 'selected' || !['pretty','compact'].includes(/** @type {string} */ (choice.id))) fail('invalid_response');
  return /** @type {import('./json-types').Format} */ (choice.id);
}

/** @param {import('./json-types').SharedContext} ctx @param {string} text */
export function send(ctx, text) {
  if (typeof ctx.desktop?.sendText !== 'function') fail('desktop_unavailable');
  return ctx.desktop.sendText(text);
}

/** @param {import('./json-types').JsonReady} task @param {import('./json-types').Format} style @param {unknown} receipt @returns {import('./json-types').JsonSubmitted} */
export function complete(task, style, receipt) {
  if (!fields(receipt,['submitted']) || receipt.submitted !== true) fail('invalid_response');
  return {status:'submitted',format:style,...task.summary,outputUtf16Units:(/** @type {string} */ (task.outputs[style])).length,inputSubmitted:true};
}

/** @param {unknown} error @param {import('./json-types').JsonMethod} method @returns {import('./json-types').JsonActionFault} */
export function sanitized(error, method) {
  /** @type {readonly import('./json-types').JsonErrorCode[]} */
  const known = ['invalid_input','invalid_json','json_limit','json_duplicate_key','json_output_too_large',
    'invalid_response','ui_unavailable','ui_busy','ui_error','desktop_unavailable','native_bridge_unavailable',
    'target_changed','target_unavailable','cancelled','timeout','input_failed'];
  /** @type {import('./json-types').JsonErrorCode} */
  let code = 'json_text_failed';
  /** @type {import('./json-types').EffectState} */
  let effectState = method === 'keyboard.text' ? 'unknown' : 'not_started';
  try {
    const reported = /** @type {import('./json-types').FaultInfo} */ (error)?.code;
    if (known.includes(/** @type {import('./json-types').JsonErrorCode} */ (reported))) code = /** @type {import('./json-types').JsonErrorCode} */ (reported);
    if (['not_started','unknown'].includes(/** @type {string} */ (/** @type {import('./json-types').FaultInfo} */ (error)?.effectState))) effectState = /** @type {import('./json-types').EffectState} */ (/** @type {import('./json-types').FaultInfo} */ (error).effectState);
    if (code === 'desktop_unavailable') effectState = 'not_started';
  } catch (_) { /* 错误 getter 的消息不外传。 */ }
  return Object.assign(new Error('JSON 文本动作未完成，请查看错误代码。'), {code,method,effectState});
}
