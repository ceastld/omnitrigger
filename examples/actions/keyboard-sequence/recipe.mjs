import {timing} from './timing.mjs';

export default async ctx => {
  const steps = ctx.input.steps;
  if (!Array.isArray(steps) || steps.length < 1 || steps.length > 32) throw new Error('需要 1–32 个输入步骤');
  for (const step of steps) {
    if (!step || !['sequence','tap','text','delay'].includes(step.kind)) throw new Error('输入步骤类型无效');
  }
  const options = steps.some(step => step.kind === 'sequence') ? await timing(ctx) : undefined;
  for (const step of steps) {
    switch (step.kind) {
      case 'sequence': await ctx.desktop.sendKeySequence(step.modifiers, step.keys, options); break;
      case 'tap': await ctx.desktop.sendKeySequence([], [step.key], {keyDownDelayMs:0}); break;
      case 'text': await ctx.desktop.sendText(step.text); break;
      case 'delay': await ctx.delay(step.milliseconds); break;
    }
  }
  return {inputSubmitted:true, steps:steps.length};
};
