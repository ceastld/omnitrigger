// 同一份 SDK 用于两个引擎。桥接仅传 JSON，不绑定引擎对象。
const nativeCall = globalThis.__hostCall;
delete globalThis.__hostCall;
const encode = JSON.stringify.bind(JSON);
const decode = JSON.parse.bind(JSON);
let calls = 0;
ctx.apiVersion = 1;
if (ctx.invocation) Object.freeze(ctx.invocation);
function call(kind, args = {}) {
    if (++calls > 32) throw new Error('单次脚本最多调用 32 次宿主 API');
    const response = decode(nativeCall(encode({ kind, ...args })));
    if (!response.ok) {
        const error = new Error(response.error.message);
        error.code = response.error.code;
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
ctx.clipboard = Object.freeze({
    readText: () => call('read_clipboard'),
    writeText: text => call('write_clipboard', { text }),
});
ctx.open = target => call('open', { target });
Object.freeze(ctx);
