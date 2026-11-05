/** @param {import('../../../crates/omnitrigger-actions/runtime/api.js').ActionContext} ctx */
export default async function main(ctx) {
    // 只等待应用创建回执，通知显示时长由应用管理。
    return await ctx.ui.notify({
        message: ctx.input.message,
        title: ctx.input.title,
        level: ctx.input.level,
        placement: ctx.input.placement,
        durationMs: ctx.input.durationMs,
    });
}
