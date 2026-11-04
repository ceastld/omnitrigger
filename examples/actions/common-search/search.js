/// <reference path="../../../tools/js-engine-probe/api.d.ts" />
/** @type {ScriptMain} */
function main(ctx) {
    const source = ctx.input.source;
    const template = ctx.input.urlTemplate === undefined ? 'https://www.google.com/search?q=%s' : ctx.input.urlTemplate;
    const limit = ctx.input.maxCodePoints === undefined ? 128 : ctx.input.maxCodePoints;
    /** @param {string} message */
    function invalid(message) {
        throw Object.assign(new Error(message), {
            code: 'invalid_input', effectState: 'not_started', method: 'script.parameters',
        });
    }
    // 只接受明确的文本来源；不因选中内容为空而使用旧剪贴板。
    if (source !== 'selection' && source !== 'text') invalid('source 需要 selection 或 text。');
    if (typeof template !== 'string' || template.length > 2048 ||
        !/^https?:\/\/[A-Za-z0-9.-]+(?::[0-9]{1,5})?\/[^\s\\\x00-\x1f]*$/.test(template) ||
        template.split('%s').length !== 2 || template.indexOf('%s') < template.indexOf('/', template.indexOf('://') + 3)) {
        invalid('urlTemplate 需要带路径的 HTTP(S) URL，并且路径或查询中恰好包含一个 %s。');
    }
    try {
        encodeURIComponent(template);
    } catch (_) {
        invalid('urlTemplate 含无效 Unicode。');
    }
    if (typeof limit !== 'number' || !Number.isInteger(limit) || limit < 1 || limit > 2048) {
        invalid('maxCodePoints 需要 1–2048 的整数。');
    }
    if (source === 'text' && (typeof ctx.input.text !== 'string' || !ctx.input.text.trim() ||
        ctx.input.text.length > 65536 || ctx.input.text.includes('\0'))) invalid('text 需要非空文本，最多 65,536 个 UTF-16 单元。');
    const text = source === 'selection' ? ctx.desktop.copySelectionText() : ctx.input.text;
    if (typeof text !== 'string' || !text.trim() || text.length > 65536 || text.includes('\0')) {
        throw Object.assign(new Error('没有可搜索的文本。'), {
            code: 'invalid_selection', effectState: 'unknown', method: 'script.selection',
        });
    }
    let encoded;
    const points = Array.from(text);
    try {
        // 完整校验后再截断，拒绝孤立代理项；字符不会被截成半个 UTF-16 代理对。
        encodeURIComponent(text);
        encoded = encodeURIComponent(points.slice(0, limit).join(''));
    } catch (_) {
        if (source === 'text') invalid('text 含无效 Unicode。');
        throw Object.assign(new Error('选中文本含无效 Unicode。'), {
            code: 'invalid_selection', effectState: 'unknown', method: 'script.selection',
        });
    }
    ctx.open(template.replace('%s', encoded));
    // 不在结果或日志中返回选中文本、搜索词或完整 URL。
    return {submitted: true, source, queryCodePoints: Math.min(points.length, limit), truncated: points.length > limit};
}
