/** @param {import('../../../crates/omnitrigger-actions/runtime/api.js').ActionContext} ctx */
export default async function main(ctx) {
    // 由共享SDK校验完整参数，只等待创建回执。
    return await ctx.ui.notify(/** @type {import('../../../crates/omnitrigger-actions/runtime/api.js').NotifyOptions} */ (ctx.input));
}
