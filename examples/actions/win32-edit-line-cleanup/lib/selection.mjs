import {removeEmptyLines,removeWhiteSpaceLines} from './transform.mjs';
/** @typedef {'remove-empty-lines'|'remove-white-space-lines'} Operation */
/** @typedef {{kind:'ready',original:string,replacement:string}|{kind:'no_selection'|'unchanged'}} Plan */
/** @param {string} code @param {'not_started'|'unknown'} [effectState] @param {string} [method] */
export function fault(code,effectState='not_started',method='selection.read_text') {
  return Object.assign(new Error('选区文本转换未完成，请检查结构化错误。'),{code,effectState,method});
}
/** @param {unknown} input @returns {Operation} */
export function operationFromInput(input) {
  if(input===undefined) return 'remove-empty-lines';
  if(input===null||typeof input!=='object'||Array.isArray(input)||Reflect.ownKeys(input).some(key=>key!=='operation')) throw fault('invalid_input');
  const operation='operation' in input?input.operation:'remove-empty-lines';
  if(operation!=='remove-empty-lines'&&operation!=='remove-white-space-lines') throw fault('invalid_input');
  return operation;
}
/** @param {unknown} input @returns {{operation:Operation,adapter:'native-edit'|'winforms-edit'}} */
export function configurationFromInput(input) {
  if(input===null||typeof input!=='object'||Array.isArray(input)
      ||Reflect.ownKeys(input).some(key=>key!=='operation'&&key!=='adapter')||!('adapter' in input)) throw fault('invalid_input','not_started','win32_edit.replace_selection');
  const adapter=input.adapter;
  if(adapter!=='native-edit'&&adapter!=='winforms-edit') throw fault('invalid_input','not_started','win32_edit.replace_selection');
  return {adapter,operation:operationFromInput('operation' in input?{operation:input.operation}:undefined)};
}
/** @param {unknown} value @returns {string} */
function completeText(value) {
  if(value===null||typeof value!=='object'||Array.isArray(value)||Reflect.ownKeys(value).length!==2
      ||!('text' in value)||!('truncated' in value)||typeof value.text!=='string'||typeof value.truncated!=='boolean') throw fault('invalid_response');
  if(value.truncated) throw fault('selection_truncated');
  if(value.text.length>8192) throw fault('selection_too_long');
  if(value.text.includes('\0')) throw fault('invalid_response');
  for(let i=0;i<value.text.length;i++) {
    const unit=value.text.charCodeAt(i);
    if(unit>=0xd800&&unit<=0xdbff){const next=value.text.charCodeAt(++i);if(!(next>=0xdc00&&next<=0xdfff))throw fault('invalid_response');}
    else if(unit>=0xdc00&&unit<=0xdfff) throw fault('invalid_response');
  }
  return value.text;
}
/** @param {unknown} snapshot @param {Operation} [operation] @returns {Plan} */
export function prepare(snapshot,operation='remove-empty-lines') {
  const original=completeText(snapshot);
  if(original==='') return {kind:'no_selection'};
  const replacement=operation==='remove-white-space-lines'?removeWhiteSpaceLines(original):removeEmptyLines(original);
  if(replacement===original) return {kind:'unchanged'};
  // 非空选区的空转换结果仍须二次确认，再交显式Edit API删除。
  // 显式Edit API上限8192 UTF16；转换可能因CRLF增长，超限在第二次读取之前拒绝。
  if(replacement.length>8192) throw fault('replacement_too_long','not_started','win32_edit.replace_selection');
  return {kind:'ready',original,replacement};
}
/** @param {Extract<Plan,{kind:'ready'}>} plan @param {unknown} snapshot */
export function confirmOriginal(plan,snapshot) {
  if(completeText(snapshot)!==plan.original) throw fault('selection_changed');
}
/** @param {unknown} response */
export function processed(response) {
  if(response===null||typeof response!=='object'||Reflect.ownKeys(response).length!==2
      ||!('messageProcessed' in response)||response.messageProcessed!==true
      ||!('effectState' in response)||response.effectState!=='unknown') throw fault('invalid_response','unknown','win32_edit.replace_selection');
  return {status:'replacement_message_processed',effectState:'unknown'};
}
/** @param {unknown} cause */
export function sendFailure(cause) {
  let code='native_bridge_failed';let state=/** @type {'not_started'|'unknown'} */ ('unknown');
  try {
    const reported=cause!==null&&typeof cause==='object'?Object.getOwnPropertyDescriptor(cause,'code')?.value:undefined;
    const effect=cause!==null&&typeof cause==='object'?Object.getOwnPropertyDescriptor(cause,'effectState')?.value:undefined;
    if(typeof reported==='string'&&/^[a-z_]{1,64}$/.test(reported)) code=reported;
    if(effect==='not_started') state='not_started';
  } catch (_) { /* 不调用异常getter，不回显正文。 */ }
  return fault(code,state,'win32_edit.replace_selection');
}
/** @param {{aborted:boolean}} signal */
export function checkCancelled(signal){if(signal.aborted) throw fault('cancelled');}
