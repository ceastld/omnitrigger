import {prepare,choose,selection,send,complete,sanitized} from './workflow.mjs';

export default async function main(ctx) {
  let method = 'script.parameters';
  try {
    const task = prepare(ctx.input);
    if (task.done) return task.value;
    method = 'ui.choose';
    const style = task.format ?? selection(await choose(ctx));
    if (style === null) return {status:'cancelled',...task.summary,inputSubmitted:false};
    method = 'keyboard.text';
    return complete(task,style,await send(ctx,task.outputs[style]));
  } catch (error) { throw sanitized(error,method); }
}
