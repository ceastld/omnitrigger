/** @param {import('../../../crates/omnitrigger-actions/runtime/api.js').ActionContext} ctx */
export default async function main(ctx) {
  const choice = await ctx.ui.choose({
    title: '选择文本风格',
    items: [
      {id:'plain', label:'纯文本', description:'不附加格式'},
      {id:'markdown', label:'Markdown', description:'保留标题与列表'},
      {id:'code', label:'代码', description:'适合代码环境'},
    ],
  });
  // 使用稳定 ID 分派后续操作；此示例只返回选择，不修改目标或剪贴板。
  return choice;
}
