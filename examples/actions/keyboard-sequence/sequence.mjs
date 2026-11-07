/// <reference path="./sdk.d.ts" />
import {timing} from './timing.mjs';
import {prepareSequence} from './prepare-keys.mjs';
/** @param {OmniTriggerActionContext} ctx */
export default async ctx => {
  const plan = prepareSequence(ctx.input);
  const options = await timing({ input:plan, desktop:ctx.desktop });
  const result = await ctx.desktop.sendKeySequence(plan.modifiers.slice(), plan.keys.slice(), options);
  return {inputSubmitted:result.submitted};
};
