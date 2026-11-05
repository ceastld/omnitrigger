/// <reference path="../../../tools/js-engine-probe/api.d.ts" />
/** @type {ScriptMain} */
function main(ctx) {
    const source = ctx.input.source;
    const template = ctx.input.urlTemplate === undefined ? 'https://www.google.com/search?q=%s' : ctx.input.urlTemplate;
    const lengthUnit = ctx.input.queryLengthUnit === undefined ? 'codepoints' : ctx.input.queryLengthUnit;
    const limit = lengthUnit === 'utf16'
        ? (ctx.input.maxUtf16Units === undefined ? 128 : ctx.input.maxUtf16Units)
        : (ctx.input.maxCodePoints === undefined ? 128 : ctx.input.maxCodePoints);
    /** @param {string} message */
    function invalid(message) {
        throw Object.assign(new Error(message), {
            code: 'invalid_input', effectState: 'not_started', method: 'script.parameters',
        });
    }
    /** @param {string} code @param {string} message */
    function pathError(code, message) {
        throw Object.assign(new Error(message), {
            code, effectState: 'not_started', method: 'fs.stat',
        });
    }
    // 与 web-search 相同：先验证参数，只复制新选区，不读取旧剪贴板。
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
    if (lengthUnit !== 'codepoints' && lengthUnit !== 'utf16') invalid('queryLengthUnit 需要 codepoints 或 utf16。');
    if ((lengthUnit === 'utf16' && Object.prototype.hasOwnProperty.call(ctx.input, 'maxCodePoints')) ||
        (lengthUnit === 'codepoints' && Object.prototype.hasOwnProperty.call(ctx.input, 'maxUtf16Units'))) {
        invalid('查询长度单位与上限字段不能混用。');
    }
    if (typeof limit !== 'number' || !Number.isInteger(limit) || limit < 1 || limit > 2048) {
        invalid(lengthUnit === 'utf16' ? 'maxUtf16Units 需要 1–2048 的整数。' : 'maxCodePoints 需要 1–2048 的整数。');
    }
    if (source === 'text' && (typeof ctx.input.text !== 'string' || !ctx.input.text.trim() ||
        ctx.input.text.length > 65536 || ctx.input.text.includes('\0'))) invalid('text 需要非空文本，最多 65,536 个 UTF-16 单元。');
    const text = source === 'selection' ? ctx.desktop.copySelectionText() : ctx.input.text;
    if (typeof text !== 'string' || !text.trim() || text.length > 65536 || text.includes('\0')) {
        throw Object.assign(new Error('没有可打开或搜索的文本。'), {
            code: 'invalid_selection', effectState: 'unknown', method: 'script.selection',
        });
    }
    try {
        // 任何分支都先检查完整文本，不能让截断隐藏孤立代理项。
        encodeURIComponent(text);
    } catch (_) {
        if (source === 'text') invalid('text 含无效 Unicode。');
        throw Object.assign(new Error('选中文本含无效 Unicode。'), {
            code: 'invalid_selection', effectState: 'unknown', method: 'script.selection',
        });
    }
    // 只辨识路径意图，不在这里复制 SDK / Windows 平台的完整路径校验。
    // trim/去引号仅用于识别并拒绝模糊形式，绝不据此改写传给 stat/open 的原值。
    const candidate = text.trim().replace(/^["']|["']$/g, '');
    const absolute = /^[A-Za-z]:[\\/]/.test(text);
    const pathLike = candidate.includes('\\') || /^[^\s/:\\]:|^[^\s/:\\]+:\/(?!\/)|^[\\/]|^\.{1,2}(?:[\\/]|$)|^~(?:[\\/]|$)|^file:|^%[^%]+%|^\$(?:env:|\{)/i.test(candidate);
    let pathKind;
    if (absolute || pathLike) {
        if (!absolute || candidate !== text) invalid('路径仅接受未加引号、没有外围空白的 Windows 普通盘符绝对路径。');
        // stat 的云端占位/拒绝访问/取消等错误原样传播；没有搜索回退。
        if (!ctx.fs || typeof ctx.fs.stat !== 'function') pathError('fs_unavailable', '当前宿主没有文件元数据接口。');
        const metadata = ctx.fs.stat(text);
        if (!metadata || typeof metadata !== 'object' || !['file', 'directory', 'missing', 'other'].includes(metadata.kind)) {
            pathError('invalid_response', '文件元数据回执无效。');
        }
        if (metadata.kind === 'other') pathError('unsupported_path_type', '目标不是可打开的普通文件或明确的目录/缺失项。');
        if (metadata.kind === 'file') {
            // 保留盘符、分隔符、Unicode 和所有原始字符，不转成 URL、不读取内容。
            ctx.open(text);
            return {submitted: true, source, operation: 'open-file'};
        }
        // 已核对的 File.Exists 分支：目录/缺失不是文件，继续原来的搜索。
        // 只有明确元数据结果走这里，任何 stat 异常都不会变成搜索。
        pathKind = metadata.kind;
    }
    if (lengthUnit === 'utf16') {
        const end = Math.min(text.length, limit);
        const last = text.charCodeAt(end - 1);
        if (text.length > limit && last >= 0xd800 && last <= 0xdbff) {
            throw Object.assign(new Error('UTF-16 查询边界会拆开代理对；请调整上限。'), {
                code: 'query_utf16_boundary', method: 'script.query',
                effectState: source === 'selection' ? 'unknown' : 'not_started',
            });
        }
        ctx.open(template.replace('%s', encodeURIComponent(text.slice(0, end))));
        return {submitted: true, source, operation: 'search', queryLengthUnit: 'utf16',
            queryUtf16Units: end, truncated: text.length > limit,
            ...(pathKind === undefined ? {} : {pathKind})};
    }
    const points = Array.from(text);
    const encoded = encodeURIComponent(points.slice(0, limit).join(''));
    ctx.open(template.replace('%s', encoded));
    return {submitted: true, source, operation: 'search', queryCodePoints: Math.min(points.length, limit), truncated: points.length > limit,
        ...(pathKind === undefined ? {} : {pathKind})};
}
