/// <reference path="../../../crates/omnitrigger-actions/runtime/script-api.d.ts" />
/** @type {ScriptMain} */
function main(ctx) {
    const input = ctx.input;
    if (Object.keys(input).some(key => !['mode', 'options'].includes(key)) ||
        !['read', 'copy'].includes(input.mode) || (input.mode === 'read' && input.options !== undefined)) {
        throw Object.assign(new Error('mode 需要 read 或 copy；只有 copy 接受 options。'), {
            code: 'invalid_input', method: 'script.parameters', effectState: 'not_started',
        });
    }
    /** @param {ScriptClipboardDataSnapshot} data */
    function summarize(data) {
        const utf16Units = data.kind === 'text'
            ? data.text.length
            : data.paths.reduce((total, path) => total + path.length, 0);
        // 空文本仍是文本；文件列表不串成字符串，不检查、枚举、打开或搜索路径。
        return {kind: data.kind, utf16Units, itemCount: data.kind === 'text' ? 1 : data.paths.length};
    }
    if (input.mode === 'copy') {
        const data = ctx.desktop.copySelectionData(input.options);
        return {...summarize(data), changed: data.changed};
    }
    return summarize(ctx.clipboard.readData());
}
