import {timing} from './timing.mjs';
export default async ctx => {
  const result = await ctx.desktop.sendKeySequence(ctx.input.modifiers, ctx.input.keys, await timing(ctx));
  return {inputSubmitted:result.submitted};
};
