export default async ctx => {
  await ctx.clipboard.writeText(ctx.input.text);
  await ctx.delay(50);
  await ctx.desktop.sendShortcut([162,86]);
  return {inputSubmitted:true};
};
