export default async ctx => {
  const selected = await ctx.desktop.copySelectionText();
  await ctx.clipboard.writeText(`(${selected})`);
  await ctx.delay(50);
  await ctx.desktop.sendShortcut([162,86]);
  return {inputSubmitted:true,selectedCharacters:selected.length};
};
