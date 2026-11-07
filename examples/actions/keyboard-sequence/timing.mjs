/// <reference path="./sdk.d.ts" />
/** @param {Pick<OmniTriggerActionContext,'input'|'desktop'>} ctx */
export async function timing(ctx) {
  if (ctx.input.keyDownDelayMs !== undefined) return {keyDownDelayMs:ctx.input.keyDownDelayMs};
  const name = (await ctx.desktop.target()).processName.toLowerCase().replace(/\.exe$/, '');
  return {keyDownDelayMs:['wps','et','wpp','wpspdf','mstsc'].includes(name) ? 10 : 0};
}
