/** @type {ScriptMain} */
export function main(ctx) {
  if (typeof ctx.desktop?.adjustVolume !== 'function') {
    throw Object.assign(new Error('当前宿主未提供音量调节 API，请更新对应运行时。'),
      {code:'runtime_unavailable',effectState:'not_started',method:'audio.volume_adjust'});
  }
  // 同步入口与Node入口复用同一SDK参数和回执合同。
  return ctx.desktop.adjustVolume(/** @type {ScriptVolumeAdjustOptions} */(ctx.input));
}
