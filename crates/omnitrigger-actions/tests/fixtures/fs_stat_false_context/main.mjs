// 独立API作者；路径仅传严格FakeHost，不在作者中读取或创建。
export function main(ctx) { return ctx.fs.stat(ctx.input.path); }
