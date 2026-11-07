/// <reference path="./sdk.d.ts" />
import {timing} from './timing.mjs';
import {prepareShortcuts} from './prepare-keys.mjs';
/** @param {OmniTriggerActionContext} ctx */
export default async ctx => {
  const plan = prepareShortcuts(ctx.input);
  const {keyDownDelayMs} = await timing({ input:plan, desktop:ctx.desktop });
  for (const step of plan.steps) {
    await ctx.desktop.sendShortcut(step.keys.slice(), {keyDownDelayMs});
    if (step.delayAfterMs !== undefined) await ctx.delay(step.delayAfterMs);
  }
  return {inputSubmitted:true, steps:plan.steps.length};
};
