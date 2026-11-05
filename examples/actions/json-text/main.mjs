import {prepare,choose,selection,send,complete,sanitized} from './workflow.mjs';

/** @param {import('./json-types').NodeContext} ctx @returns {Promise<import('./json-types').FormatActionResult>} */
export default async function main(ctx) {
  /** @type {import('./json-types').JsonMethod} */
  let method = 'script.parameters';
  try {
    const task = prepare(ctx.input);
    if (task.done) return task.value;
    method = 'ui.choose';
    const style = task.format ?? selection(await choose(ctx));
    if (style === null) return {status:'cancelled',...task.summary,inputSubmitted:false};
    method = 'keyboard.text';
    return complete(task,style,await send(ctx,/** @type {string} */ (task.outputs[style])));
  } catch (error) { throw sanitized(error,method); }
}
