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
        else if (kind === 'native') error.effectState = args.method === 'win32_edit.replace_selection' ? 'unknown' : 'not_started';
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
const captureProcess = createProcessCaptureApi(params => call('capture', params)).capture;
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
    if (method === 'win32_edit.replace_selection' && calls >= 32) throw Object.assign(new Error('单次脚本最多调用32次宿主API。'),{code:'desktop_call_limit',effectState:'not_started',method});

    if (!nativeContext || nativeContext.error) {
        if (method === 'win32_edit.replace_selection') throw Object.assign(new Error('Win32 Edit替换需要应用本机桥接。'),{code:'native_bridge_unavailable',effectState:'not_started',method});

        if (method === 'audio.volume_adjust') throw Object.assign(new Error('音量调节需要应用本机能力桥接。'),{code:'native_bridge_unavailable',effectState:'not_started',method});
        if (method === 'display.brightness_step') throw Object.assign(new Error('亮度步进需要应用本机能力桥接。'), {code:'native_bridge_unavailable',effectState:'not_started',method});
        if (['history.list','history.get','history.append','history.remove'].includes(method)) {
            throw Object.assign(new Error('剪贴板历史调用失败；请检查错误代码。'), {code:'history_unavailable',effectState:'not_started',method});
        }
        if (method === 'clipboard.read_data' || method === 'selection.copy_data' || method === 'explorer.context') {
            throw Object.assign(new Error('数据选区 API 需要应用本机能力桥接。'), {code:'native_bridge_unavailable',effectState:'not_started',method});
        }
        const cause = nativeContext?.error;
        const ui = method === 'ui.choose' || method === 'ui.notify' || method === 'ui.open_history' || method === 'ui.toggle_history';
        if (method === 'fs.stat' || method === 'fs.read_directory') throw Object.assign(new Error('文件状态 API 需要本机宿主。'), {code:'fs_unavailable',effectState:'not_started',method});
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
ctx.openFileInParent = function (target) {
    if (arguments.length !== 1 || typeof target !== 'string' || !/^[A-Za-z]:[\\/]/.test(target)
        || target.length > 32766 || target.trim() !== target || /[\u0000-\u001f\u007f-\u009f]/.test(target)
        || /[:<>"|?*]/.test(target.slice(3))) {
        throw Object.assign(new Error('需要明确的普通本地文件路径。'), {code:'invalid_input',effectState:'not_started',method:'system.open_file_in_parent'});
    }
    try { encodeURIComponent(target); }
    catch (_) { throw Object.assign(new Error('路径含无效 Unicode。'), {code:'invalid_input',effectState:'not_started',method:'system.open_file_in_parent'}); }
    return call('open_file_in_parent', {target});
};
Object.freeze(ctx);
