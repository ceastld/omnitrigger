/** @param {import('../../../crates/omnitrigger-actions/runtime/api').ActionContext} ctx */
export default async function (ctx) {
  const tree = await ctx.browser.call('bookmarks.getTree', [], {
    ...(ctx.input.profile === undefined ? {} : { profile: ctx.input.profile }),
    timeoutMs: 10000,
  });
  const stack = [...tree];
  let bookmarks = 0;
  let foldersIncludingRoots = 0;
  while (stack.length) {
    const node = stack.pop();
    if (typeof node.url === 'string') bookmarks++;
    else foldersIncludingRoots++;
    if (Array.isArray(node.children)) for (const child of node.children) stack.push(child);
  }
  // 完整树只在脚本内处理；动作历史与最终结果不保存书签标题和 URL。
  return { bookmarks, foldersIncludingRoots };
}
