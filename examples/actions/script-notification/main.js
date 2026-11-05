/** @type {ScriptMain} */
function main(ctx) {
    // 通知正文和样式由 call.json 的 input 编辑；SDK 严格校验，不强制转换参数。
    return ctx.ui.notify({
        message: ctx.input.message,
        title: ctx.input.title,
        level: ctx.input.level,
        placement: ctx.input.placement,
        durationMs: ctx.input.durationMs,
    });
}
