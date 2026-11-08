// 独立API作者，无业务菜单、窗口或剪贴板调用；实际打开完全由严格FakeHost接收。
export function main(ctx) {
  return ctx.input.parent ? ctx.openFileInParent(ctx.input.target) : ctx.open(ctx.input.target);
}
