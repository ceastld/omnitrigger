/// <reference path="./sdk.d.ts" />
import {timing} from './timing.mjs';
import {prepareRecipe} from './prepare-recipe.mjs';

/** @typedef {OmniTriggerActionContext} RecipeContext */
/** @param {RecipeContext} ctx */
export default async ctx => {
  const plan = prepareRecipe(ctx.input);
  const options = plan.hasSequence
    ? await timing({ input: { keyDownDelayMs: plan.requestedDelay }, desktop: ctx.desktop }) : undefined;
  for (const step of plan.steps) {
    switch (step.kind) {
      case 'sequence': await ctx.desktop.sendKeySequence([...step.modifiers], [...step.keys], options); break;
      case 'tap': await ctx.desktop.sendKeySequence([], [step.key], {keyDownDelayMs:0}); break;
      case 'text': await ctx.desktop.sendText(step.text); break;
      case 'delay': await ctx.delay(step.milliseconds); break;
    }
  }
  return {inputSubmitted:plan.steps.some(step => step.kind !== 'delay'), steps:plan.steps.length};
};
