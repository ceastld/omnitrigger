/// <reference path="./sdk/api.d.ts" />
/** @type {ScriptMain} */
function main(ctx) {
    /** @returns {never} */
    function invalid() {
        throw Object.assign(new Error('需要字面箭头文本、明确粘贴键和 0–1000 毫秒的按下延迟。'), {
            code: 'invalid_input', effectState: 'not_started', method: 'script.parameters',
        });
    }
    const input = ctx.input;
    if (!input || typeof input !== 'object' || Array.isArray(input)) invalid();
    const text = input.text;
    const pasteKey = input.pasteKey;
    const keyDownDelayMs = input.keyDownDelayMs;
    if (Object.keys(input).some(key => !['text', 'pasteKey', 'keyDownDelayMs'].includes(key))
        || (text !== '\\Leftarrow ' && text !== '\\Rightarrow ')
        || (pasteKey !== 'ctrl-v' && pasteKey !== 'shift-insert')
        || typeof keyDownDelayMs !== 'number' || !Number.isInteger(keyDownDelayMs)
        || keyDownDelayMs < 0 || keyDownDelayMs > 1000) invalid();
    const keys = pasteKey === 'ctrl-v' ? [162, 86] : [160, 45];
    // target() 核验本次捕获的原目标，不重新选择前台。
    const target = ctx.desktop.target();
    if (!target.isForeground) {
        throw Object.assign(new Error('原目标已不在前台，未写入剪贴板。'), {
            code: 'target_changed', effectState: 'not_started', method: 'target.info',
        });
    }
    ctx.clipboard.writeText(text);
    let phase = 'delay';
    try {
        // 明确的 50ms 等待，不代替剪贴板序号轮询或目标应用完成确认。
        ctx.delay(50);
        phase = 'keyboard.shortcut';
        const receipt = ctx.desktop.sendShortcut(keys, {keyDownDelayMs});
        if (receipt?.submitted !== true) throw new Error('粘贴输入未获确认。');
        return {clipboardWritten: true, pasteInputSubmitted: true, pasteKey};
    } catch (cause) {
        // 即使粘贴自身被拒绝，前面的 Clipboard 写入已经发生。
        throw Object.assign(new Error('剪贴板已经写入，后续粘贴未确认；请先检查结果，避免重复执行。'), {
            code: 'clipboard_written_paste_unconfirmed', effectState: 'unknown', method: phase, cause,
        });
    }
}
