import {prepare,history,sample,extract,complete,sanitized} from './workflow.mjs';

export default async function main(ctx) {
  let method = 'script.parameters';
  let copied = false;
  try {
    const task = prepare(ctx.input);
    method = 'history.append';
    const api = history(ctx); // 先确认历史接口存在，避免无法保存时仍发送复制。
    method = task.mode === 'copy' ? 'selection.copy_data' : 'clipboard.read_data';
    const data = await sample(ctx,task);
    copied = task.mode === 'copy';
    const value = extract(data,task);
    method = 'history.append';
    return complete(await api.append({text:value.text}),value,task);
  } catch (error) { throw sanitized(error,method,copied); }
}
