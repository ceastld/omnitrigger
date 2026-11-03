export default async function (ctx) {
  const target = await ctx.desktop.target();
  // 发送时窗口已失效或前台已是其他窗口，后续输入会被拒绝。
  await ctx.desktop.sendShortcut([160, 36]); // 左 Shift + Home
  await ctx.delay(80);
  await ctx.desktop.sendShortcut([8]); // Backspace
  return {processName: target.processName, inputSubmitted: true};
}
