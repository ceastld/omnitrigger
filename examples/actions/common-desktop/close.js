/// <reference path="../../../tools/js-engine-probe/api.d.ts" />
/** @type {ScriptMain} */
function main(ctx) {
    const processes = ctx.input.ctrlWProcesses;
    const delay = ctx.input.keyDownDelayMs;
    // 在读取目标、提交输入之前校验整个配置。列表匹配区分大小写。
    if (!Array.isArray(processes) || processes.length > 64 || processes.some(name =>
        typeof name !== 'string' || !name || name.length > 260 || name.trim() !== name ||
        /[\\/,:\x00-\x1f]/.test(name) || /\.exe$/i.test(name))) {
        throw Object.assign(new Error('ctrlWProcesses 需要最多 64 个进程名，不带路径或 .exe 后缀。'), {
            code: 'invalid_input', effectState: 'not_started', method: 'script.parameters',
        });
    }
    if (delay !== undefined && (typeof delay !== 'number' || !Number.isInteger(delay) || delay < 0 || delay > 1000)) {
        throw Object.assign(new Error('keyDownDelayMs 需要 0–1000 毫秒的整数。'), {
            code: 'invalid_input', effectState: 'not_started', method: 'script.parameters',
        });
    }
    const target = ctx.desktop.target();
    const processName = target.processName.replace(/\.exe$/i, '');
    const ctrlW = processes.includes(processName);
    const keyDownDelayMs = delay === undefined
        ? (['wps', 'et', 'wpp', 'wpspdf', 'mstsc'].includes(processName.toLowerCase()) ? 10 : 0)
        : delay;
    // 原目标变化、失效或 API 失败时直接传播错误，不重试或改用另一种关闭方式。
    ctx.desktop.sendKeySequence(ctrlW ? [162] : [164], ctrlW ? [87] : [115], {keyDownDelayMs});
    return {processName: target.processName, operation: ctrlW ? 'ctrl_w' : 'alt_f4', submitted: true};
}
