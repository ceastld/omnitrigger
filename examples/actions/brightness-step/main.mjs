/** @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx) {
  if (typeof ctx.desktop?.stepBrightness !== 'function') {
    throw Object.assign(new Error('当前宿主未提供亮度步进 API，请更新对应运行时。'), {code:'runtime_unavailable',effectState:'not_started',method:'display.brightness_step'});
  }
  // 原样交给共享SDK，不能丢弃未知字段或替作者补默认策略。
  return await ctx.desktop.stepBrightness(/** @type {import('./sdk/api').BrightnessStepOptions} */(ctx.input));
}
