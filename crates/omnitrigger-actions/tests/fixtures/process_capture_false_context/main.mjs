// 独立API测试作者：唯一能力调用是process.capture，不是任何业务父菜单的替身。
export function main(ctx) {
  return ctx.process.capture(ctx.input.program, ctx.input.args, ctx.input.options);
}
