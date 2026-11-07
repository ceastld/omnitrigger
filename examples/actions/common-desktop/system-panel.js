/// <reference path="./sdk/api.d.ts" />
/** @type {ScriptMain} */
function main(ctx) {
    if (!ctx.input || typeof ctx.input !== 'object' || Array.isArray(ctx.input)
        || Object.keys(ctx.input).length !== 0) {
        throw Object.assign(new Error('此固定动作只接受空对象参数。'), {
            code: 'invalid_input', effectState: 'not_started', method: 'script.parameters',
        });
    }
    // 时序由原生 API 根据捕获目标解析，保持同一原目标。
    const receipt = ctx.desktop.sendShortcut([91, 65], {
        timing: {
            key_down_delay_ms: 0,
            process_overrides: {wps: 10, et: 10, wpp: 10, wpspdf: 10, mstsc: 10},
        },
    });
    if (!receipt || receipt.submitted !== true) {
        throw Object.assign(new Error('未取得明确的按键提交回执，结果未知；不自动重试。'), {
            code: 'input_unconfirmed', effectState: 'unknown', method: 'keyboard.shortcut',
        });
    }
    return {inputSubmitted: true};
}
