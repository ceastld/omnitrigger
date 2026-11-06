import {removeEmptyLines,removeWhiteSpaceLines} from './transform.mjs';
/** @typedef {'remove-empty-lines'|'remove-white-space-lines'} Operation */
/** @typedef {{kind:'ready',original:string,replacement:string}|{kind:'no_selection'|'unchanged'|'empty_output_skipped'}} Plan */
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
  // 空结果不提交，不用空sendText表示删除整段。
  if(replacement==='') return {kind:'empty_output_skipped'};
  // 首版只提交最多128个UTF16单元；不截断，且在第二次读取之前拒绝。
  if(replacement.length>128) throw fault('replacement_too_long','not_started','keyboard.text');
  return {kind:'ready',original,replacement};
}
/** @param {Extract<Plan,{kind:'ready'}>} plan @param {unknown} snapshot */
export function confirmOriginal(plan,snapshot) {
  if(completeText(snapshot)!==plan.original) throw fault('selection_changed');
}
/** @param {unknown} response */
export function submitted(response) {
  if(response===null||typeof response!=='object'||Reflect.ownKeys(response).length!==1||!('submitted' in response)||response.submitted!==true) throw fault('invalid_response','unknown','keyboard.text');
  return {status:'replacement_submitted'};
}
/** @param {unknown} cause */
export function sendFailure(cause) {
  const error=cause!==null&&typeof cause==='object'?cause:{};
  const code='code' in error&&typeof error.code==='string'&&/^[a-z_]{1,64}$/.test(error.code)?error.code:'native_bridge_failed';
  const state='effectState' in error&&error.effectState==='not_started'?'not_started':'unknown';
  return fault(code,state,'keyboard.text');
}
/** @param {{aborted:boolean}} signal */
export function checkCancelled(signal){if(signal.aborted) throw fault('cancelled');}
