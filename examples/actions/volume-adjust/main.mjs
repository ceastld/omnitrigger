/** @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx) {
  if (typeof ctx.desktop?.adjustVolume !== 'function') {
    throw Object.assign(new Error('当前宿主未提供音量调节 API，请更新对应运行时。'),
      {code:'runtime_unavailable',effectState:'not_started',method:'audio.volume_adjust'});
  }
  // 整体交给SDK校验，不补步长、不丢弃额外字段、不改用媒体键。
  return await ctx.desktop.adjustVolume(/** @type {import('./sdk/api').VolumeAdjustOptions} */(ctx.input));
}
