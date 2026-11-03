export default async ctx => {
  await ctx.desktop.sendText(ctx.input.text);
  return {inputSubmitted:true};
};
