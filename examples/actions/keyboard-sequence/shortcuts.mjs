import {timing} from './timing.mjs';
export default async ctx => {
  const steps = ctx.input.steps;
  if (!Array.isArray(steps) || steps.length < 1 || steps.length > 16) throw new Error('需要 1–16 个快捷键步骤');
  const options = await timing(ctx);
  for (const step of steps) {
    await ctx.desktop.sendShortcut(step.keys, options);
    if (step.delayAfterMs !== undefined) await ctx.delay(step.delayAfterMs);
  }
  return {inputSubmitted:true, steps:steps.length};
};
