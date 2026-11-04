/// <reference path="../../../tools/js-engine-probe/api.d.ts" />
/** @param {ScriptContext} ctx */
function main(ctx) {
    const waitMs = ctx.input.waitMs === undefined ? 80 : ctx.input.waitMs;
    const requestedDelay = ctx.input.keyDownDelayMs;
    /** @param {string} name @param {unknown} value @param {number} maximum */
    function integer(name, value, maximum) {
        if (typeof value !== 'number' || !Number.isInteger(value) || value < 0 || value > maximum) {
            throw Object.assign(new Error(`${name} 需要 0–${maximum} 毫秒的整数。`), {
                code: 'invalid_input', effectState: 'not_started', method: 'script.parameters',
            });
        }
    }
    // 参数错误在任何输入之前报告，避免选中一半后才发现等待参数无效。
    integer('waitMs', waitMs, 60000);
    if (requestedDelay !== undefined) integer('keyDownDelayMs', requestedDelay, 1000);
    const target = ctx.desktop.target();
    const processName = target.processName.toLowerCase().replace(/\.exe$/, '');
    const keyDownDelayMs = requestedDelay === undefined
        ? (['wps', 'et', 'wpp', 'wpspdf', 'mstsc'].includes(processName) ? 10 : 0)
        : requestedDelay;
    ctx.desktop.sendKeySequence([160], [36], {keyDownDelayMs}); // 左 Shift + Home
    ctx.delay(waitMs);
    ctx.desktop.sendKeySequence([], [8], {keyDownDelayMs}); // Backspace
    return {processName: target.processName, inputSubmitted: true};
}
