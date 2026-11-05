import {analyze, format} from './json.mjs';

const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const fields = (value, names) => object(value) && Reflect.ownKeys(value).length === names.length
  && names.every(name => Object.hasOwn(value, name));
const fail = code => { throw Object.assign(new Error('JSON 配方参数或结果无效。'), {code}); };

export function prepare(input) {
  if (!object(input) || !['inspect','send','choose-send'].includes(input.mode)) fail('invalid_input');
  const mode = input.mode;
  if (!fields(input, mode === 'send' ? ['mode','text','format'] : ['mode','text'])) fail('invalid_input');
  const result = analyze(input.text);
  if (mode === 'inspect') return {done:true,value:{status:'inspected',...result.summary,inputSubmitted:false}};
  const outputs = mode === 'send' ? {[input.format]:format(result.tokens, input.format)}
    : {compact:format(result.tokens,'compact'),pretty:format(result.tokens,'pretty')};
  return {done:false,mode,outputs,summary:result.summary,format:mode === 'send' ? input.format : undefined};
}

export function choose(ctx) {
  if (typeof ctx.ui?.choose !== 'function') fail('ui_unavailable');
  return ctx.ui.choose({title:'选择 JSON 排版',items:[{id:'pretty',label:'缩进排版'},{id:'compact',label:'紧凑排版'}]});
}

export function selection(choice) {
  if (fields(choice,['status','reason']) && choice.status === 'cancelled'
      && ['dismissed','timeout'].includes(choice.reason)) return null;
  if (!fields(choice,['status','id']) || choice.status !== 'selected' || !['pretty','compact'].includes(choice.id)) fail('invalid_response');
  return choice.id;
}

export function send(ctx, text) {
  if (typeof ctx.desktop?.sendText !== 'function') fail('desktop_unavailable');
  return ctx.desktop.sendText(text);
}

export function complete(task, style, receipt) {
  if (!fields(receipt,['submitted']) || receipt.submitted !== true) fail('invalid_response');
  return {status:'submitted',format:style,...task.summary,outputUtf16Units:task.outputs[style].length,inputSubmitted:true};
}

export function sanitized(error, method) {
  const known = ['invalid_input','invalid_json','json_limit','json_duplicate_key','json_output_too_large',
    'invalid_response','ui_unavailable','ui_busy','ui_error','desktop_unavailable','native_bridge_unavailable',
    'target_changed','target_unavailable','cancelled','timeout','input_failed'];
  let code = 'json_text_failed';
  let effectState = method === 'keyboard.text' ? 'unknown' : 'not_started';
  try {
    const reported = error?.code;
    if (known.includes(reported)) code = reported;
    if (['not_started','unknown'].includes(error?.effectState)) effectState = error.effectState;
    if (code === 'desktop_unavailable') effectState = 'not_started';
  } catch (_) { /* 错误 getter 的消息不外传。 */ }
  return Object.assign(new Error('JSON 文本动作未完成，请查看错误代码。'), {code,method,effectState});
}
