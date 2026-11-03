export default async function (ctx) {
  return await ctx.desktop.sendShortcut(ctx.input.keys, {timing:ctx.input.timing ?? {}});
}
