/** @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx) {
  // 原样交给共享SDK，不能丢弃未知字段或替作者补默认策略。
  return await ctx.desktop.stepBrightness(/** @type {import('./sdk/api').BrightnessStepOptions} */(ctx.input));
}
