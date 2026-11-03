export default async ctx => {
  await ctx.desktop.sendShortcut([35]);
  await ctx.delay(1);
  await ctx.desktop.sendShortcut([13]);
  await ctx.delay(1);
  return {inputSubmitted:true};
};
