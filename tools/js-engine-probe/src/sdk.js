// 同一份 SDK 用于两个引擎。桥接仅传 JSON，不绑定引擎对象。
const nativeCall = globalThis.__hostCall;
delete globalThis.__hostCall;
const encode = JSON.stringify.bind(JSON);
const decode = JSON.parse.bind(JSON);
let calls = 0;
let nativeId = 0;
const nativeContext = ctx.__nativeContext;
delete ctx.__nativeContext;
ctx.apiVersion = 1;
if (ctx.invocation) Object.freeze(ctx.invocation);
function call(kind, args = {}) {
    if (++calls > 32) throw new Error('单次脚本最多调用 32 次宿主 API');
    const response = decode(nativeCall(encode({ kind, ...args })));
    if (!response.ok) {
        const error = new Error(response.error.message);
        error.code = response.error.code;
        if (response.error.effect_state) error.effectState = response.error.effect_state;
        if (response.error.method) error.method = response.error.method;
        throw error;
    }
    return response.value;
}
ctx.fs = Object.freeze({
    readText: path => call('read_text', { path }),
    writeText: (path, text, options = {}) => call('write_text', { path, text, overwrite: options.overwrite ?? false }),
});
ctx.process = Object.freeze({
    run: (program, args = [], options = {}) => call('run', { program, args, timeout_ms: options.timeoutMs ?? 1000 }),
    start: (program, args = []) => call('start', { program, args }),
});
function desktopCall(method, params) {
    if (!nativeContext || nativeContext.error) {
        const cause = nativeContext?.error;
        throw Object.assign(new Error(cause?.message ?? '桌面 API 需要 OmniTrigger 本次动作的目标上下文。'), {
            code:cause?.code ?? 'desktop_unavailable', effectState:cause?.effect_state ?? 'not_started', method,
        });
    }
    return call('native', {connection:nativeContext.connection, id:++nativeId, method, params});
}
const desktopApis = createDesktopApi(desktopCall);
ctx.desktop = desktopApis.desktop;
ctx.clipboard = nativeContext ? desktopApis.clipboard : Object.freeze({
    readText: () => call('read_clipboard'),
    writeText: (text, options = {}) => {
        if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).length) {
            throw Object.assign(new Error('独立 CLI 的剪贴板写入不支持选项。'), {code:'invalid_input',effectState:'not_started',method:'clipboard.write'});
        }
        return call('write_clipboard', {text});
    },
});
ctx.open = target => call('open', { target });
Object.freeze(ctx);
