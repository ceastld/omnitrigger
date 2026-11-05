/** @type {ScriptMain} */
export function main(ctx) {
  if (typeof ctx.desktop?.stepBrightness !== 'function') {
    throw Object.assign(new Error('当前宿主未提供亮度步进 API，请更新对应运行时。'), {code:'runtime_unavailable',effectState:'not_started',method:'display.brightness_step'});
  }
  // 参数与回执均由同一共享SDK验证；不重试或补偿。
  return ctx.desktop.stepBrightness(/** @type {ScriptBrightnessStepOptions} */(ctx.input));
}
