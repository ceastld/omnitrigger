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
function call(kind, args = {}, serialized) {
    if (++calls > 32) throw new Error('单次脚本最多调用 32 次宿主 API');
    const response = decode(nativeCall(serialized ?? encode({ kind, ...args })));
    if (!response.ok) {
        const error = new Error(response.error.message);
        error.code = response.error.code;
        if (response.error.effect_state) error.effectState = response.error.effect_state;
        else if (kind === 'native') error.effectState = 'not_started';
        if (response.error.method) error.method = response.error.method;
        if (response.error.required_permissions) error.requiredPermissions = response.error.required_permissions;
        throw error;
    }
    return response.value;
}
ctx.fs = Object.freeze({
    readText: path => call('read_text', { path }),
    writeText: (path, text, options = {}) => call('write_text', { path, text, overwrite: options.overwrite ?? false }),
});
function captureProcess(program, args = [], options = {}) {
    let dispatched = false;
    const fail = code => { throw Object.assign(new Error('程序输出采集未完成，请检查错误代码。'), {code}); };
    const unicode = (value, nul = false) => {
        if (typeof value !== 'string' || (!nul && value.includes('\0'))) return false;
        for (let i = 0; i < value.length; i++) {
            const unit = value.charCodeAt(i);
            if (unit >= 0xd800 && unit <= 0xdbff) {
                const next = value.charCodeAt(++i);
                if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
            } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
        }
        return true;
    };
    const local = value => unicode(value) && /^[A-Za-z]:[\\/]/.test(value)
        && !value.slice(3).split(/[\\/]/).some(part => part === '.' || part === '..');
    try {
        if (typeof program !== 'string' || program.length > 32766 || !local(program) || !program.toLowerCase().endsWith('.exe') || !Array.isArray(args) || args.length > 128
            || Reflect.ownKeys(args).length !== args.length + 1 || args.some((arg, i) => !Object.hasOwn(args,i) || typeof arg !== 'string' || arg.length > 32766 || !unicode(arg))
            || !options || typeof options !== 'object' || Array.isArray(options)
            || Reflect.ownKeys(options).some(key => !['timeoutMs','allowNonZero','cwd'].includes(key))) fail('invalid_input');
        for (let i = 0; i < args.length; i++) if (!Object.hasOwn(args,i)) fail('invalid_input');
        if (program.length + args.reduce((length,arg) => length+arg.length,0) + 3*args.length + 3 > 32767) fail('invalid_input');
        const {timeoutMs = 10000, allowNonZero = false, cwd} = options;
        if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 60000 || typeof allowNonZero !== 'boolean'
            || (Object.hasOwn(options,'cwd') && !local(cwd))
            || Object.keys(options).some(key => options[key] === undefined)) fail('invalid_input');
        // 与本机 MSVCRT 引号规则计算同一命令行长度，不把参数拼给 shell。
        const quoted = value => {
            const parts = ['"'];
            let slashes = 0;
            for (const character of value) {
                if (character === '\\') { slashes++; continue; }
                parts.push('\\'.repeat(character === '"' ? slashes*2+1 : slashes), character);
                slashes = 0;
            }
            parts.push('\\'.repeat(slashes*2), '"');
            return parts.join('');
        };
        if ([program,...args].map(quoted).join(' ').length + 1 > 32767) fail('invalid_input');
        dispatched = true;
        const value = call('capture', {program,args:[...args],timeout_ms:timeoutMs,allow_non_zero:allowNonZero,
            ...(cwd === undefined ? {} : {cwd})});
        if (!value || typeof value !== 'object' || Array.isArray(value)
            || Reflect.ownKeys(value).length !== 3 || !['code','stdout','stderr'].every(key => Object.hasOwn(value,key))
            || !Number.isInteger(value.code) || value.code < -2147483648 || value.code > 2147483647
            || value.stdout?.length > 65536 || value.stderr?.length > 65536
            || !unicode(value.stdout,true) || !unicode(value.stderr,true)) fail('invalid_response');
        let bytes = 0;
        for (const text of [value.stdout,value.stderr]) for (const character of text) {
            const point = character.codePointAt(0);
            bytes += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
            if (bytes > 65536) fail('invalid_response');
        }
        if (value.code !== 0 && !allowNonZero) fail('process_exit_nonzero');
        return {code:value.code,stdout:value.stdout,stderr:value.stderr};
    } catch (cause) {
        let code = 'process_capture_failed';
        let effectState = dispatched ? 'unknown' : 'not_started';
        try {
            const known = ['invalid_input','invalid_arguments','invalid_response','request_too_large','unsupported_platform',
                'process_setup_failed','process_start_failed','process_read_failed','process_wait_failed','process_clock_error',
                'process_timeout','process_output_too_large','process_invalid_utf8','process_exit_nonzero','process_cleanup_failed'];
            if (known.includes(cause?.code)) code = cause.code;
            if (cause?.effectState === 'not_started' || ['invalid_arguments','request_too_large','unsupported_platform',
                'process_setup_failed','process_start_failed'].includes(code)) effectState = 'not_started';
        } catch (_) { /* 不回显异常对象、getter、参数或采集输出。 */ }
        throw Object.assign(new Error('程序输出采集未完成，请检查错误代码。'), {code,effectState,method:'process.capture'});
    }
}
ctx.process = Object.freeze({
    run: (program, args = [], options = {}) => call('run', { program, args, timeout_ms: options.timeoutMs ?? 1000 }),
    capture: captureProcess,
    start: (program, args = []) => call('start', { program, args }),
});
ctx.delay = milliseconds => {
    if (!Number.isInteger(milliseconds) || milliseconds < 0 || milliseconds > 60_000) {
        throw Object.assign(new Error('等待时间必须为 0–60000 毫秒的整数。'), {
            code:'invalid_input', effectState:'not_started', method:'delay',
        });
    }
    call('delay', {milliseconds});
};
function desktopCall(method, params) {
    if (!nativeContext || nativeContext.error) {
        if (['history.list','history.get','history.append','history.remove'].includes(method)) {
            throw Object.assign(new Error('剪贴板历史调用失败；请检查错误代码。'), {code:'history_unavailable',effectState:'not_started',method});
        }
        if (method === 'clipboard.read_data' || method === 'selection.copy_data') {
            throw Object.assign(new Error('数据选区 API 需要应用本机能力桥接。'), {code:'native_bridge_unavailable',effectState:'not_started',method});
        }
        const cause = nativeContext?.error;
        const ui = method === 'ui.choose' || method === 'ui.notify';
        if (method === 'fs.stat') throw Object.assign(new Error('文件状态 API 需要本机宿主。'), {code:'fs_unavailable',effectState:'not_started',method});
        const code = ui && cause?.code === 'desktop_unavailable' ? 'ui_unavailable' : cause?.code;
        throw Object.assign(new Error(cause?.message ?? (method.startsWith('browser.') ? '浏览器 API 需要 OmniTrigger 应用宿主。' : ui ? '此 UI API 需要 OmniTrigger 应用界面。' : '桌面 API 需要 OmniTrigger 本次动作的目标上下文。')), {
            code:code ?? (method.startsWith('browser.') ? 'browser_unavailable' : ui ? 'ui_unavailable' : 'desktop_unavailable'), effectState:cause?.effect_state ?? 'not_started', method,
        });
    }
    const request = {connection:nativeContext.connection, id:nativeId+1, method, params};
    const serialized = encode({kind:'native',...request});
    const tooLarge = () => {throw Object.assign(new Error('本机调用请求超过 512 KiB。'), {code:'invalid_input',effectState:'not_started',method});};
    if (serialized.length > 512*1024) tooLarge();
    if (/[^\x00-\x7f]/.test(serialized)) {
        let bytes = 0;
        for (const character of serialized) {
            const point = character.codePointAt(0);
            bytes += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
            if (bytes > 512*1024) tooLarge();
        }
    }
    nativeId++;
    return call('native', request, serialized);
}
const desktopApis = createDesktopApi(desktopCall);
ctx.fs = Object.freeze({...ctx.fs, ...desktopApis.fs});
ctx.desktop = desktopApis.desktop;
ctx.ui = desktopApis.ui;
ctx.browser = createBrowserApi((method, args, options, profiles) => {
    let value;
    try { value = desktopCall(profiles ? 'browser.profiles' : 'browser.call', profiles ? {options} : {method,args,options}); }
    catch (error) { if (!profiles && error?.method === 'browser.call') error.method = method; throw error; }
    if (profiles) return value;
    if (value?.has_value === true && Object.hasOwn(value,'value')) return value.value;
    if (value?.has_value === false) return undefined;
    throw Object.assign(new Error('浏览器调用回执格式无效。'), {code:'invalid_response',effectState:'unknown',method});
});
ctx.clipboard = nativeContext ? desktopApis.clipboard : Object.freeze({
    readData: desktopApis.clipboard.readData,
    readText: () => call('read_clipboard'),
    writeText: (text, options = {}) => {
        if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).length) {
            throw Object.assign(new Error('独立 CLI 的剪贴板写入不支持选项。'), {code:'invalid_input',effectState:'not_started',method:'clipboard.write'});
        }
        return call('write_clipboard', {text});
    },
});
// 独立 CLI 仍暴露历史 API，但没有应用桥接时明确不可用，不回退系统剪贴板。
ctx.clipboard = Object.freeze({...ctx.clipboard, history:createClipboardHistoryApi(desktopCall)});
ctx.open = target => call('open', { target });
Object.freeze(ctx);
