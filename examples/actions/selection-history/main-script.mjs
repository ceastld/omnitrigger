import {prepare,history,sample,extract,complete,sanitized} from './workflow.mjs';

export function main(ctx) {
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
