/** @type {ScriptMain} */
function main(ctx) {
    // 由共享SDK校验完整参数，不能在入口丢弃未知字段。
    return ctx.ui.notify(/** @type {ScriptNotifyOptions} */ (ctx.input));
}
