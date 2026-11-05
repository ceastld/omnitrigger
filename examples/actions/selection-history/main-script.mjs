import {prepare,history,sample,extract,complete,sanitized} from './workflow.mjs';

/** @param {import('./selection-types').SyncContext} ctx @returns {import('./selection-types').ActionResult} */
export function main(ctx) {
  /** @type {import('./selection-types').Method} */
  let method = 'script.parameters';
  let copied = false;
  try {
    const task = prepare(ctx.input);
    method = 'history.append';
    const api = history(ctx);
    method = task.mode === 'copy' ? 'selection.copy_data' : 'clipboard.read_data';
    const data = sample(ctx,task);
    copied = task.mode === 'copy';
    const value = extract(data,task);
    method = 'history.append';
    return complete(api.append({text:value.text}),value,task);
  } catch (error) { throw sanitized(error,method,copied); }
}
