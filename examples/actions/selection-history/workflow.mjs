// 显式单次采样到历史；没有监控、文件读取或隐含的旧内容回退。
const maximumId = '18446744073709551615';
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const fields = (value, names) => object(value) && Reflect.ownKeys(value).length === names.length
  && names.every(name => Object.hasOwn(value, name));
const fail = code => { throw Object.assign(new Error('文本历史采样参数或结果无效。'), {code}); };
const decimal = (value, zero = false) => typeof value === 'string' && /^(0|[1-9][0-9]*)$/.test(value)
  && (zero || value !== '0') && (value.length < 20 || value.length === 20 && value <= maximumId);

function validText(value) {
  if (typeof value !== 'string' || value.length > 65536 || value.includes('\0')) return false;
  for (let index = 0; index < value.length; index++) {
    const unit = value.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(++index);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
  }
  return true;
}

export function prepare(input) {
  if (!object(input) || !['read','copy'].includes(input.mode)) fail('invalid_input');
  const mode = input.mode;
  if (mode === 'read') {
    if (!fields(input,['mode'])) fail('invalid_input');
    return {mode,options:undefined,allowUnchanged:false};
  }
  if (!fields(input,['mode']) && !fields(input,['mode','options'])) fail('invalid_input');
  if (!Object.hasOwn(input,'options')) return {mode,options:undefined,allowUnchanged:false};
  const value = input.options;
  const allowed = ['timeoutMs','pollIntervalMs','settleMs','allowUnchanged'];
  if (!object(value) || Reflect.ownKeys(value).some(key => !allowed.includes(key))) fail('invalid_input');
  const {timeoutMs=600,pollIntervalMs=15,settleMs=30,allowUnchanged=false} = value;
  // 与既有 SDK 使用相同范围；全部错误先于任何读取/复制/历史写入。
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 5000
      || !Number.isInteger(pollIntervalMs) || pollIntervalMs < 1 || pollIntervalMs > 1000 || pollIntervalMs > timeoutMs
      || !Number.isInteger(settleMs) || settleMs < 0 || settleMs > 1000 || typeof allowUnchanged !== 'boolean'
      || Object.keys(value).some(key => value[key] === undefined)) fail('invalid_input');
  return {mode,options:{timeoutMs,pollIntervalMs,settleMs,allowUnchanged},allowUnchanged};
}

export function history(ctx) {
  const api = ctx.clipboard?.history;
  if (typeof api?.append !== 'function') fail('history_unavailable');
  return api;
}

export function sample(ctx, task) {
  if (task.mode === 'read') {
    if (typeof ctx.clipboard?.readData !== 'function') fail('native_bridge_unavailable');
    return ctx.clipboard.readData();
  }
  if (typeof ctx.desktop?.copySelectionData !== 'function') fail('native_bridge_unavailable');
  return task.options === undefined ? ctx.desktop.copySelectionData() : ctx.desktop.copySelectionData(task.options);
}

export function extract(data, task) {
  if (!object(data) || !['text','files'].includes(data.kind)) fail('invalid_response');
  const expected = ['kind',data.kind === 'text' ? 'text' : 'paths','sequenceNumber',
    ...(task.mode === 'copy' ? ['changed'] : [])];
  if (!fields(data,expected) || !Number.isInteger(data.sequenceNumber) || data.sequenceNumber < 0 || data.sequenceNumber > 0xffffffff
      || task.mode === 'copy' && typeof data.changed !== 'boolean') fail('invalid_response');
  if (data.kind === 'files') {
    if (!Array.isArray(data.paths) || data.paths.length > 64 || Reflect.ownKeys(data.paths).length !== data.paths.length+1
        || data.paths.some(path => !validText(path) || !path.length)
        || data.paths.reduce((units,path) => units+path.length,0) > 65536) fail('invalid_response');
    fail('history_text_required');
  }
  if (!validText(data.text)) fail('invalid_response');
  if (task.mode === 'copy' && data.changed === false && !task.allowUnchanged) fail('selection_unchanged');
  return {text:data.text,...(task.mode === 'copy' ? {changed:data.changed} : {})};
}

export function complete(receipt, value, task) {
  if (!fields(receipt,['id','revision','inserted']) || !decimal(receipt.id) || !decimal(receipt.revision,true)
      || typeof receipt.inserted !== 'boolean') fail('invalid_response');
  return {operation:task.mode === 'copy' ? 'copy-to-history' : 'read-to-history',saved:true,
    inserted:receipt.inserted,utf16Units:value.text.length,...(task.mode === 'copy' ? {changed:value.changed} : {})};
}

export function sanitized(error, method, copied) {
  const known = ['invalid_input','invalid_response','history_unavailable','history_text_required','history_corrupt','history_busy',
    'history_conflict','history_failed','history_id_exhausted','history_revision_exhausted','selection_unchanged',
    'clipboard_busy','clipboard_error','clipboard_too_large','invalid_clipboard_data','unsupported_clipboard_format',
    'target_changed','target_unavailable','cancelled','timeout','native_bridge_unavailable'];
  let code = 'selection_history_failed';
  let effectState = ['selection.copy_data','history.append'].includes(method) ? 'unknown' : 'not_started';
  try {
    const reported = error?.code;
    if (known.includes(reported)) code = reported;
    const state = error?.effectState;
    if (['not_started','unknown'].includes(state)) effectState = state;
  } catch (_) { /* 不外传正文或错误 getter。 */ }
  if (['history_unavailable','native_bridge_unavailable'].includes(code) && !copied) effectState = 'not_started';
  // 历史写入被取消也不撤销已经成功发出的复制，不把整次动作说成未开始。
  if (copied) effectState = 'unknown';
  return Object.assign(new Error('文本采样到历史未完成，请查看错误代码。'),{code,method,effectState});
}
