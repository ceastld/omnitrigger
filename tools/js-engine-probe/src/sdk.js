// 同一份 SDK 用于两个引擎。桥接仅传 JSON，不绑定引擎对象。
const nativeCall = globalThis.__hostCall;
delete globalThis.__hostCall;
const encode = JSON.stringify.bind(JSON);
const decode = JSON.parse.bind(JSON);
let calls = 0;
let nativeId = 0;
const nativeContext = ctx.__nativeContext;
delete ctx.__nativeContext;
// Rust运行层给出明确策略；兼容旧生产者时只识别连接/非Standalone错误，不把任意对象当应用桥。
const legacyBridgeBinding = !!nativeContext && typeof nativeContext === 'object' &&
    (Object.hasOwn(nativeContext, 'connection') || Object.hasOwn(nativeContext, 'browser_async_connection') ||
     typeof nativeContext.error?.code === 'string' && nativeContext.error.code !== 'desktop_unavailable');
const localDesktopAllowed = ctx.__localDesktopAllowed !== false && !legacyBridgeBinding;
delete ctx.__localDesktopAllowed;
const legacyDesktopMethods = Object.freeze({open:'system.open',open_file_in_parent:'system.open_file_in_parent',
    read_clipboard:'clipboard.read',write_clipboard:'clipboard.write',capture:'process.capture'});
const actionsScope = nativeContext?.actions_scope === true;
const nativeContextAvailable = nativeContext && nativeContext.native_context_available !== false;
ctx.apiVersion = 1;
if (ctx.invocation) Object.freeze(ctx.invocation);
function call(kind, args = {}, serialized) {
    if (!localDesktopAllowed && Object.hasOwn(legacyDesktopMethods, kind)) {
        throw Object.assign(new Error('应用桥调用禁止退回宿主本地桌面API。'),
            {code:'local_desktop_forbidden',effectState:'not_started',method:legacyDesktopMethods[kind]});
    }
    if (++calls > 32) throw new Error('单次脚本最多调用 32 次宿主 API');
    const response = decode(nativeCall(serialized ?? encode({ kind, ...args })));
    if (!response.ok) {
        const error = new Error(response.error.message);
        error.code = response.error.code;
        if (response.error.effect_state) error.effectState = response.error.effect_state;
        else if (kind === 'native' && args.method === 'process.capture') error.effectState = 'unknown';
        else if (kind === 'native') error.effectState = (args.method === 'win32_edit.replace_selection' || args.method === 'clipboard.write_files' || ['window.set_rect','window.drag','target.handoff_pointing_root','raishot.capture_result','raishot.release_result'].includes(args.method) || typeof args.method === 'string' && args.method.startsWith('raishot.resident.')) ? 'unknown' : 'not_started';
        if (response.error.method) error.method = response.error.method;
        if (response.error.required_permissions) error.requiredPermissions = response.error.required_permissions;
        if (kind === 'native' && args.method === 'display.brightness_step' && response.error.method === args.method
            && ['cancelled','timeout'].includes(error.code) && error.effectState === 'unknown'
            && response.error.brightness_receipt !== undefined) error.brightnessReceipt = response.error.brightness_receipt;
        throw error;
    }
    return response.value;
}
ctx.fs = Object.freeze({
    readText: path => call('read_text', { path }),
    writeText: (path, text, options = {}) => call('write_text', { path, text, overwrite: options.overwrite ?? false }),
});
const captureProcess = createProcessCaptureApi(params => localDesktopAllowed ? call('capture', params) : desktopCall('process.capture', params)).capture;
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
    const fileResult = method === 'raishot.capture_result' || method === 'raishot.release_result';
    const actionHint = method === 'ui.action_hint';
    const opening = method === 'system.open' || method === 'system.open_file_in_parent';
    const statPath = method === 'fs.stat' || method === 'fs.sort_logical_paths';
    const browserMethod = method === 'browser.call' || method === 'browser.profiles';
    if (browserMethod && !nativeContext?.connection && !nativeContext?.error) throw Object.assign(new Error('浏览器调用需要本次应用桥。'),
        {code:'browser_unavailable',effectState:'not_started',method});
    if (statPath && !nativeContext?.connection && !nativeContext?.error) throw Object.assign(new Error('文件状态查询需要本次应用桥。'),
        {code:'fs_unavailable',effectState:'not_started',method});
    if (opening && !nativeContext?.connection && !nativeContext?.error) throw Object.assign(new Error('打开请求需要本次应用桥；不会本地打开。'),
        {code:'native_bridge_unavailable',effectState:'not_started',method});
    if ((method === 'process.capture' || opening || statPath) && calls >= 32) throw Object.assign(new Error('单次脚本最多调用32次宿主API。'),
        {code:'desktop_call_limit',effectState:'not_started',method});
    const windowCall = ['window.geometry','window.set_rect','pointer.state','pointer.root','target.handoff_pointing_root','window.drag'].includes(method);
    if (actionHint && !actionsScope) throw Object.assign(new Error('版本提示需要当前服务调用作用域。'),{code:'ui_hint_scope_expired',effectState:'not_started',method});
    if ((method === 'win32_edit.replace_selection' || method === 'clipboard.write_files' || (method.startsWith('raishot.resident.') || fileResult) || method === 'state.read' || method === 'state.compare_and_swap' || windowCall) && calls >= 32) throw Object.assign(new Error('单次脚本最多调用32次宿主API。'),{code:'desktop_call_limit',effectState:'not_started',method});

    if (!nativeContext || nativeContext.error || !opening && !statPath && !browserMethod && !fileResult && !['actions.stop_other_instances','state.read','state.compare_and_swap','ui.action_hint','process.capture'].includes(method) && !nativeContextAvailable) {
        if (windowCall) throw Object.assign(new Error('窗口接口需要应用本机上下文。'),{code:'native_bridge_unavailable',effectState:'not_started',method});
        if (method === "actions.stop_other_instances") throw Object.assign(new Error("动作控制需要当前服务调用作用域。"),{code:"actions_unavailable",effectState:"not_started",method});
        if (method.startsWith('raishot.resident.') || fileResult) throw Object.assign(new Error('RaiShot 常驻接口需要应用本机桥接。'),{code:'native_bridge_unavailable',effectState:'not_started',method});
        if (method === 'clipboard.write_files') throw Object.assign(new Error('文件列表写入需要应用本机桥接。'),{code:'native_bridge_unavailable',effectState:'not_started',method});
        if (method === 'win32_edit.replace_selection') throw Object.assign(new Error('Win32 Edit替换需要应用本机桥接。'),{code:'native_bridge_unavailable',effectState:'not_started',method});

        if (method === 'audio.volume_adjust') throw Object.assign(new Error('音量调节需要应用本机能力桥接。'),{code:'native_bridge_unavailable',effectState:'not_started',method});
        if (method === 'display.brightness_step') throw Object.assign(new Error('亮度步进需要应用本机能力桥接。'), {code:'native_bridge_unavailable',effectState:'not_started',method});
        if (['history.list','history.get','history.append','history.remove'].includes(method)) {
            throw Object.assign(new Error('剪贴板历史调用失败；请检查错误代码。'), {code:'history_unavailable',effectState:'not_started',method});
        }
        if (method === 'clipboard.read_data' || method === 'selection.copy_data' || method === 'explorer.context') {
            throw Object.assign(new Error('数据选区 API 需要应用本机能力桥接。'), {code:'native_bridge_unavailable',effectState:'not_started',method});
        }
        if (method === 'process.capture') throw Object.assign(new Error('程序输出采集需要本次应用桥；不会本地启动。'),
            {code:'native_bridge_unavailable',effectState:'not_started',method});
        const cause = nativeContext?.error;
        const ui = method === 'ui.choose' || method === 'ui.notify' || method === 'ui.open_history' || method === 'ui.toggle_history' || actionHint;
        if (method === 'fs.stat' || method === 'fs.read_directory' || method === 'fs.sort_logical_paths') throw Object.assign(new Error('文件状态 API 需要本机宿主。'), {code:'fs_unavailable',effectState:'not_started',method});
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
ctx.state = createStateApi((method, params) => {
    if (!actionsScope) throw Object.assign(new Error('当前宿主没有动作状态作用域。'),{code:'state_unavailable',effectState:'not_started',method});
    return desktopCall(method,params);
});
ctx.actions = createActionsApi((method, params) => {
    if (!actionsScope) throw Object.assign(new Error('当前宿主没有ActionService调用作用域。'),{code:'actions_unavailable',effectState:'not_started',method});
    return desktopCall(method,params);
});
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
}, undefined, asyncBrowserInvoke);
ctx.clipboard = !localDesktopAllowed ? desktopApis.clipboard : Object.freeze({
    readData: desktopApis.clipboard.readData,
    writeFiles: desktopApis.clipboard.writeFiles,
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
function legacyOpenThroughBridge(method, target) {
    if (typeof target !== 'string') throw Object.assign(new Error('打开目标必须为字符串。'),
        {code:'invalid_arguments',effectState:'not_started',method});
    const result = desktopCall(method, {target});
    if (!result || typeof result !== 'object' || Array.isArray(result) ||
        Reflect.ownKeys(result).length !== 1 || result.submitted !== true) {
        throw Object.assign(new Error('打开请求回执未确认；不会重试。'),
            {code:'invalid_response',effectState:'unknown',method});
    }
    // 旧ctx.open/openFileInParent的成功返回值为null，不把桥回执改成新的作者合同。
    return null;
}
ctx.open = target => localDesktopAllowed ? call('open', {target}) : legacyOpenThroughBridge('system.open', target);
ctx.openFileInParent = function (target) {
    if (arguments.length !== 1 || typeof target !== 'string' || !/^[A-Za-z]:[\\/]/.test(target)
        || target.length > 32766 || target.trim() !== target || /[\u0000-\u001f\u007f-\u009f]/.test(target)
        || /[:<>"|?*]/.test(target.slice(3))) {
        throw Object.assign(new Error('需要明确的普通本地文件路径。'), {code:'invalid_input',effectState:'not_started',method:'system.open_file_in_parent'});
    }
    try { encodeURIComponent(target); }
    catch (_) { throw Object.assign(new Error('路径含无效 Unicode。'), {code:'invalid_input',effectState:'not_started',method:'system.open_file_in_parent'}); }
    return localDesktopAllowed ? call('open_file_in_parent', {target}) : legacyOpenThroughBridge('system.open_file_in_parent', target);
};

// 请求与完成只传 JSON；Promise resolve/reject 从不离开当前引擎线程。
const nativeAsyncCall = globalThis.__hostCallAsync;
const nativeAsyncPoll = globalThis.__hostPollAsync;
delete globalThis.__hostCallAsync;
delete globalThis.__hostPollAsync;
delete globalThis.__hostAsyncPending;
const browserPending = new Map();
function asyncFault(cause, method) {
    const error = new Error(cause?.message ?? '异步浏览器请求未确认。');
    error.code = cause?.code ?? 'invalid_response';
    error.effectState = cause?.effect_state ?? 'unknown';
    error.method = cause?.method ?? method;
    if (cause?.required_permissions) error.requiredPermissions = cause.required_permissions;
    return error;
}
function browserAsyncCall(method, params) {
    if (!nativeContext?.browser_async_connection || typeof nativeAsyncCall !== 'function' || typeof nativeAsyncPoll !== 'function') {
        throw Object.assign(new Error('当前父宿主没有异步浏览器连接。'),
            {code:'browser_async_unavailable',effectState:'not_started',method});
    }
    if (calls >= 32) throw Object.assign(new Error('单次脚本最多调用32次宿主API。'),
        {code:'desktop_call_limit',effectState:'not_started',method});
    const id = nativeId + 1;
    const serialized = encode({id,method,params});
    let bytes=0;
    for (const character of serialized) {
        const point=character.codePointAt(0);
        bytes += point<0x80?1:point<0x800?2:point<0x10000?3:4;
    }
    if (id>128 || bytes>512*1024) throw Object.assign(new Error('异步请求标识或大小超过限制。'),
        {code:'invalid_input',effectState:'not_started',method});
    calls++; nativeId=id;
    return new Promise((resolve,reject)=>{
        let reply;
        try { reply=decode(nativeAsyncCall(serialized)); }
        catch (_) { reject(asyncFault(null,method)); return; }
        if (reply?.ok !== true) { reject(asyncFault(reply?.error,method)); return; }
        if (reply.id !== id) { reject(asyncFault(null,method)); return; }
        browserPending.set(id,{resolve,reject,method});
    });
}
function pumpBrowserAsync() {
    let count=0;
    for (;;) {
        const serialized = nativeAsyncPoll?.();
        if (serialized === undefined || serialized === 'null') return count;
        let reply;
        try { reply=decode(serialized); }
        catch (_) { throw asyncFault(null,'browser.call'); }
        const pending=browserPending.get(reply?.id);
        if (!pending || typeof reply.ok !== 'boolean') throw asyncFault(null,'browser.call');
        browserPending.delete(reply.id);
        count++;
        if (reply.ok) pending.resolve(reply.value);
        else pending.reject(asyncFault(reply.error,pending.method));
    }
}
// 重用共同 browser-api 的校验/参数快照，不复制浏览器请求解析器。
async function asyncBrowserInvoke(method,args,options,profiles) {
    const wireMethod=profiles?"browser.profiles":"browser.call";
    try {
        const value=await browserAsyncCall(wireMethod,profiles?{options}:{method,args,options});
        if (profiles) return value;
        if (value?.has_value === true && Object.hasOwn(value,"value")) return value.value;
        if (value?.has_value === false) return undefined;
        throw asyncFault(null,method);
    } catch (error) {
        if (!profiles && error?.method === "browser.call") error.method=method;
        throw error;
    }
}
function makeScriptRunner() {
    function serializeResult(result) {
        const output=encode(result);
        if (typeof output !== 'string') throw new Error('入口必须返回 JSON 结果');
        return encode({kind:'value',value:decode(output)});
    }
    const runner=entry=>{
        const result=entry(ctx);
        return result && typeof result.then === 'function'
            ? Promise.resolve(result).then(value=>serializeResult(value===undefined?null:value)) : serializeResult(result);
    };
    Object.defineProperties(runner,{
        __pollAsync:{value:pumpBrowserAsync},
        __pendingAsync:{value:()=>browserPending.size},
    });
    return runner;
}

Object.freeze(ctx);
