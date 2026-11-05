/** @type {ScriptMain} */
export function main(ctx) {
  // 参数与回执均由同一共享SDK验证；不重试或补偿。
  return ctx.desktop.stepBrightness(/** @type {ScriptBrightnessStepOptions} */(ctx.input));
}
