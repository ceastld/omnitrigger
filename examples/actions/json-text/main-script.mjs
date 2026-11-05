import {prepare,choose,selection,send,complete,sanitized} from './workflow.mjs';

/** @param {import('./json-types').SyncContext} ctx @returns {import('./json-types').FormatActionResult} */
export function main(ctx) {
  /** @type {import('./json-types').JsonMethod} */
  let method = 'script.parameters';
  try {
    const task = prepare(ctx.input);
    if (task.done) return task.value;
    method = 'ui.choose';
    const style = task.format ?? selection(choose(ctx));
    if (style === null) return {status:'cancelled',...task.summary,inputSubmitted:false};
    method = 'keyboard.text';
    return complete(task,style,send(ctx,/** @type {string} */ (task.outputs[style])));
  } catch (error) { throw sanitized(error,method); }
}
