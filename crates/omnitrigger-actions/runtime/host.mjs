// OmniTrigger JavaScript host API v1. Trusted local code; this is not a sandbox.
import { spawn } from 'node:child_process';
import { writeFileSync, renameSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
import { createConnection } from 'node:net';

const LIMIT = 64 * 1024;
const controller = new AbortController();
const children = new Set();
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
let request;
let replied = false;
let browserClient;
let browserConnection;
let browserDeadline;
const browserPending = new Set();
const nativeSockets = new Set();
const nativeMethods = new Set(['state.read','state.compare_and_swap','actions.stop_other_instances','system.open','system.open_file_in_parent','process.capture','history.list','history.get','history.append','history.remove','fs.stat','fs.read_directory','ui.choose','ui.notify','ui.open_history','ui.toggle_history','browser.profiles','browser.call','target.info','target.context','target.capture_foreground','target.keyboard.shortcut','target.keyboard.text',
  'keyboard.shortcut','keyboard.sequence','keyboard.text','win32_edit.replace_selection','raishot.resident.find','raishot.resident.region','selection.copy','selection.copy_data','selection.read_text','explorer.context','clipboard.read','clipboard.read_data','clipboard.write','clipboard.write_files','window.action','display.brightness_step','audio.volume_adjust']);
let nativePending;
let nativePendingMethod;
let nativeId = 0;
const fault = (code, message) => Object.assign(new Error(message), { code,
  ...(['cancelled', 'timeout'].includes(code) ? {effectState:'unknown'} : {}),
});
const stop = (reason) => {
  if (!controller.signal.aborted) controller.abort(reason);
  for (const child of children) child.kill();
  for (const socket of nativeSockets) socket.destroy();
};
const initial = new Promise((resolve, reject) => {
  lines.on('line', (line) => {
    if (!request) {
      try {
        if (Buffer.byteLength(line) > 96 * 1024) throw fault('invalid_request', '动作请求过大。');
        request = JSON.parse(line);
        resolve(request);
      } catch (error) { reject(error); }
    } else if (line === 'cancel') {
      stop(fault('cancelled', '动作已取消；已完成的操作不会撤销。'));
    }
  });
  lines.on('close', () => {
    if (!replied) {
      const error = fault('cancelled', '宿主连接已关闭。');
      stop(error);
      if (!request) reject(error);
    }
  });
});

function checkCancelled() {
  if (controller.signal.aborted) throw controller.signal.reason;
}

function browserOptions(options) {
  if (!options || typeof options !== 'object' || Array.isArray(options)
      || Object.keys(options).some(key => !['profile', 'timeoutMs'].includes(key))
      || (options.profile !== undefined && (typeof options.profile !== 'string' || !options.profile || options.profile.length > 128))
      || (options.timeoutMs !== undefined && (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1 || options.timeoutMs > 60_000))) {
    throw fault('invalid_input', '浏览器选项仅支持 profile 和 1–60000 毫秒的 timeoutMs。');
  }
  return { ...options, timeoutMs: Math.max(1, Math.min(options.timeoutMs ?? 10_000, browserDeadline - Date.now())), signal: controller.signal };
}

async function connectedBrowser() {
  checkCancelled();
  if (!request.browser) throw Object.assign(fault('browser_unavailable', '请先安装并启动浏览器连接器。'), { effectState: 'not_started' });
  if (!browserConnection) {
    browserConnection = (async () => {
      const { connect } = await import(pathToFileURL(request.browser.sdk_path).href);
      checkCancelled();
      const client = await connect({ configPath: request.browser.config_path,
        connectTimeoutMs: Math.max(1, Math.min(5000, browserDeadline - Date.now())) });
      browserClient = client;
      if (controller.signal.aborted) { client.close(); checkCancelled(); }
      return client;
    })();
  }
  return browserConnection;
}

function browserRequest(method, args, options = {}, profiles = false) {
  if (browserPending.size >= 8) return Promise.reject(Object.assign(fault('busy', '单次动作最多同时等待 8 个浏览器调用。'), {effectState:'not_started'}));
  const operation = (async () => {
    checkCancelled();
    browserOptions(options);
    const client = await connectedBrowser();
    checkCancelled();
    return profiles ? client.profiles(browserOptions(options)) : client.call(method, args, browserOptions(options));
  })();
  browserPending.add(operation);
  // 不替脚本吞掉错误；同时收尾未 await 的 Promise，避免进程外泄漏。
  operation.then(() => browserPending.delete(operation), () => browserPending.delete(operation));
  return operation;
}

const browser = createBrowserApi(browserRequest, fn => async (...args) => fn(...args));

function nativeCall(method, params) {
  const actionControl = method === 'actions.stop_other_instances' || method === 'state.read' || method === 'state.compare_and_swap';
  const historyUi = method === 'ui.open_history' || method === 'ui.toggle_history';
  const opening = method === 'system.open' || method === 'system.open_file_in_parent';
  const capture = method === 'process.capture';
  const rai = method.startsWith('raishot.resident.');
  const edit = method === 'win32_edit.replace_selection' || rai || method === 'clipboard.write_files';
  const volume = method === 'audio.volume_adjust';
  const brightness = method === 'display.brightness_step' || volume;
  const stat = method === 'fs.stat' || method === 'fs.read_directory';
  const dataRead = method === 'clipboard.read_data' || method === 'selection.read_text' || method === 'explorer.context';
  const dataCopy = method === 'selection.copy_data';
  const data = dataRead || dataCopy;
  const historyRead = method === 'history.list' || method === 'history.get';
  const historyWrite = method === 'history.append' || method === 'history.remove';
  const history = historyRead || historyWrite;
  const fail = (code, message, effectState = 'not_started') => Object.assign(fault(code, rai ? 'RaiShot 常驻请求未获完整确认；请检查错误代码和效果状态。' : edit ? (method === 'clipboard.write_files' ? '文件列表剪贴板写入未获完整确认；请检查错误代码和效果状态。' : 'Win32 Edit选区替换未获完整确认；请检查错误代码和效果状态。') : volume ? '音量调节未获完整确认；请检查错误代码和效果状态。' : brightness ? '亮度步进未获完整确认；请检查结构化回执。' : capture ? '程序输出采集未完成，请检查错误代码。' : stat ? '本地路径状态查询失败；请检查错误代码。' : data ? '剪贴板或选区数据调用失败；请检查错误代码。' : history ? '剪贴板历史调用失败；请检查错误代码。' : message), {effectState:stat || dataRead || historyRead ? 'not_started' : effectState, method});
  if (!nativeMethods.has(method)) return Promise.reject(fail('invalid_input', '未提供此本机调用方法。'));
  if ((historyUi || stat || data || history || capture || brightness || edit || actionControl) && controller.signal.aborted) return Promise.reject(fail(controller.signal.reason?.code === 'timeout' ? 'timeout' : 'cancelled'));
  checkCancelled();
  const ui = method === 'ui.choose' || method === 'ui.notify' || historyUi;
  const unavailable = ui && request.native_error?.code === 'desktop_unavailable' ? 'ui_unavailable' : request.native_error?.code;
  if (!request.native || !actionControl && request.native_context_available === false) return Promise.reject(fail(actionControl ? 'actions_unavailable' : capture || brightness || edit ? 'native_bridge_unavailable' : history ? 'history_unavailable' : data ? 'native_bridge_unavailable' : stat ? 'fs_unavailable' : unavailable ?? (ui ? 'ui_unavailable' : 'desktop_unavailable'), request.native_error?.message ?? (ui ? '此 UI API 需要 OmniTrigger 应用界面。' : '此宿主没有提供本次调用的桌面上下文。')));
  if (nativePending) return Promise.reject(fail(stat ? 'fs_busy' : method === 'ui.toggle_history' && ['ui.choose','ui.open_history','ui.toggle_history'].includes(nativePendingMethod) ? 'ui_busy' : (method === 'ui.notify' || historyUi) && nativePendingMethod === method ? 'ui_busy' : 'desktop_busy', '请 await 上一个本机调用，避免并发。'));
  if (nativeId >= 128) return Promise.reject(fail('desktop_call_limit', '单次动作最多调用 128 次桌面 API。'));
  const id = ++nativeId;
  let frame;
  // 在本次调用中固定请求；连接建立后的发送不再读取调用方的可变数组或时序对象。
  try { frame = JSON.stringify({token:request.native.token, id, method, params}) + '\n'; }
  catch { return Promise.reject(fail('invalid_input', '本机调用参数无法编码。')); }
  const operation = new Promise((resolve, reject) => {
    let settled = false;
    let sent = false;
    let chunks = '';
    let captureBytes = 0;
    const captureDecoder = capture ? new TextDecoder('utf-8', {fatal:true}) : undefined;
    let methodTimer;
    const socket = createConnection({host:'127.0.0.1', port:request.native.port});
    nativeSockets.add(socket);
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(methodTimer);
      controller.signal.removeEventListener('abort', aborted);
      nativeSockets.delete(socket);
      socket.destroy();
      if (error) reject(error); else resolve(value);
    };
    const aborted = () => {
      const reason = controller.signal.reason;
      if (opening && reason) { reason.method = method; reason.effectState = sent ? 'unknown' : 'not_started'; }
      if (opening || capture || brightness || edit || actionControl) return finish(fail(reason?.code === 'timeout' ? 'timeout' : 'cancelled', undefined, sent ? 'unknown' : 'not_started'));
      if (method === 'ui.notify' || historyUi || stat || data || history || capture || brightness || edit || actionControl) {
        if (typeof reason?.method !== 'string') reason.method = method;
        reason.effectState = stat || dataRead || historyRead ? 'not_started' : sent ? 'unknown' : 'not_started';
      }
      finish(reason);
    };
    controller.signal.addEventListener('abort', aborted, {once:true});
    if (!capture) socket.setEncoding('utf8');
    const timeout = capture ? Math.min(params.timeout_ms + 1500, 61500) : opening || method === 'ui.choose' || brightness ? 61000 : dataCopy ? 8000 : 5000;
    socket.setTimeout(timeout, () => finish(fail(method === 'ui.notify' || historyUi || stat || data || history || capture || brightness || edit || actionControl ? 'timeout' : 'desktop_timeout', '本机调用未在等待时限内确认；不会自动重试。', sent ? 'unknown' : 'not_started')));
    // 这些方法的总传输时限不被零碎数据延长，外层取消/时限仍优先生效。
    if (historyUi || opening || stat || data || history || capture || brightness || edit || actionControl) methodTimer = setTimeout(() => finish(fail('timeout', undefined, sent ? 'unknown' : 'not_started')), timeout);
    socket.once('connect', () => {
      if (controller.signal.aborted) return aborted();
      sent = true;
      socket.write(frame);
    });
    socket.on('data', chunk => {
      if (capture) {
        captureBytes += chunk.length;
        if (captureBytes > 512 * 1024 + 1) return finish(fail('invalid_response', undefined, 'unknown'));
        try { chunk = captureDecoder.decode(chunk, {stream:true}); }
        catch { return finish(fail('invalid_response', undefined, 'unknown')); }
      }
      chunks += chunk;
      if (Buffer.byteLength(chunks) > 512 * 1024 + 1) return finish(fail(edit ? 'invalid_response' : 'desktop_protocol', '桌面回执超过上限。', 'unknown'));
      if (!chunks.endsWith('\n')) return;
      if (capture) {
        try { chunks += captureDecoder.decode(); }
        catch { return finish(fail('invalid_response', undefined, 'unknown')); }
      }
      try {
        const response = JSON.parse(chunks);
        if (response.id !== id || typeof response.ok !== 'boolean') throw new Error('回执身份不匹配');
        if ((opening || stat || history || capture || brightness || edit || actionControl) && (Reflect.ownKeys(response).length !== 3 || !Object.hasOwn(response, response.ok ? 'value' : 'error'))) throw new Error();
        if (response.ok) finish(undefined, response.value);
        else {
          const error = response.error;
          if (!error || typeof error.code !== 'string' || typeof error.message !== 'string') throw new Error('缺少错误');
          if ((opening || stat || history || capture || brightness || edit || actionControl) && ((error.method !== undefined && error.method !== method)
              || (error.effect_state !== undefined && !['not_started','unknown'].includes(error.effect_state)))) throw new Error();
          const failure = fail(error.code, error.message, error.effect_state ?? 'unknown');
          if (!stat && !history && !capture && Array.isArray(error.required_permissions)) failure.requiredPermissions = error.required_permissions;
          finish(failure);
        }
      } catch { finish(fail(edit ? 'invalid_response' : 'desktop_protocol', '桌面回执格式无效；业务结果未确认。', 'unknown')); }
    });
    socket.once('error', error => finish(fail(stat ? (sent ? 'fs_error' : 'fs_unavailable') : 'desktop_connection', error.message, sent ? 'unknown' : 'not_started')));
    socket.once('close', () => finish(fail(stat ? 'fs_error' : 'desktop_connection', '桌面连接已关闭；不会重试。', sent ? 'unknown' : 'not_started')));
  });
  nativePending = operation;
  nativePendingMethod = method;
  const completed = () => {nativePending = undefined; nativePendingMethod = undefined;};
  operation.then(completed, completed);
  return operation;
}

// 只经应用桥接打开目标；不调用Node spawn/run/start，不解释命令行。
async function openTargetWithMethod(method, ...args) {
  const fail = (code, effectState) => Object.assign(new Error('打开请求未确认，请检查错误代码。'),
    {code, effectState, method});
  const target = args[0];
  if (args.length !== 1 || typeof target !== 'string' || !target.length || target.length > 32766
      || target.trim() !== target || /[\u0000-\u001f\u007f-\u009f]/.test(target)) {
    throw fail('invalid_input', 'not_started');
  }
  if (method === 'system.open_file_in_parent'
      && (!/^[A-Za-z]:[\\/]/.test(target) || /[:<>"|?*]/.test(target.slice(3)))) {
    throw fail('invalid_input', 'not_started');
  }
  for (let index = 0; index < target.length; index++) {
    const unit = target.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = target.charCodeAt(++index);
      if (!(next >= 0xdc00 && next <= 0xdfff)) throw fail('invalid_input', 'not_started');
    } else if (unit >= 0xdc00 && unit <= 0xdfff) throw fail('invalid_input', 'not_started');
  }
  // 协议/绝对路径白名单由与轻量open相同的Rust校验独立执行，不从用户文本猜程序或参数。
  if (controller.signal.aborted) throw fail(controller.signal.reason?.code === 'timeout' ? 'timeout' : 'cancelled', 'not_started');
  if (!request.native) throw fail('native_bridge_unavailable', 'not_started');
  let result;
  try { result = await nativeCall(method, {target}); }
  catch (error) {
    let code = 'native_bridge_failed', effect = 'unknown';
    try {
      const aliases = {desktop_busy:'open_busy',desktop_call_limit:'native_call_limit',
        desktop_timeout:'timeout',desktop_protocol:'invalid_response',desktop_connection:'native_bridge_failed'};
      const known = ['invalid_input','open_failed','open_unavailable','open_busy','cancelled','timeout',
        'invalid_response','native_bridge_unavailable','native_bridge_failed','native_call_limit'];
      const reported = aliases[error?.code] ?? error?.code;
      const fileCodes = ['path_not_local','path_unavailable','access_denied','fs_error','open_file_required','open_directory_required'];
      if (known.includes(reported) || (method === 'system.open_file_in_parent' && fileCodes.includes(reported))) code = reported;
      if (error?.effectState === 'not_started') effect = 'not_started';
    } catch (_) { /* 不读取或回显异常getter正文。 */ }
    throw fail(code, effect);
  }
  try {
    if (result === null || typeof result !== 'object' || Array.isArray(result)
        || Reflect.ownKeys(result).length !== 1 || !Object.hasOwn(result, 'submitted')
        || result.submitted !== true) throw new Error();
  } catch (_) { throw fail('invalid_response', 'unknown'); }
  return {submitted:true};
}

function openTarget(...args) { return openTargetWithMethod('system.open', ...args); }
function openFileInParent(...args) { return openTargetWithMethod('system.open_file_in_parent', ...args); }

const desktopApis = createDesktopApi(nativeCall);
// Node 保持 Promise 契约；共享定义中的同步校验异常也转为拒绝。
const asyncApi = api => Object.freeze(Object.fromEntries(Object.entries(api).map(([name, fn]) => [name, async (...args) => fn(...args)])));
const desktop = asyncApi(desktopApis.desktop);
const clipboard = Object.freeze({...asyncApi(desktopApis.clipboard), history:asyncApi(createClipboardHistoryApi(nativeCall))});
const ui = asyncApi(desktopApis.ui);
const fs = asyncApi(desktopApis.fs);

function delay(milliseconds) {
  checkCancelled();
  if (!Number.isInteger(milliseconds) || milliseconds < 0 || milliseconds > 60_000) {
    throw fault('invalid_input', '等待时间必须为 0–60000 毫秒的整数。');
  }
  return new Promise((resolve, reject) => {
    const done = () => { controller.signal.removeEventListener('abort', aborted); resolve(); };
    const timer = setTimeout(done, milliseconds);
    const aborted = () => { clearTimeout(timer); reject(controller.signal.reason); };
    controller.signal.addEventListener('abort', aborted, { once: true });
  });
}

function validateProcess(program, args, options, allowedOptions) {
  checkCancelled();
  if (typeof program !== 'string' || !program.trim() || program.includes('\0')
      || !Array.isArray(args) || args.length > 128
      || args.some(arg => typeof arg !== 'string' || arg.includes('\0') || arg === '--omni-selected-text-once' || arg.startsWith('--omni-selected-text-once=') || arg === '--omni-explorer-context-once' || arg.startsWith('--omni-explorer-context-once='))
      || !options || typeof options !== 'object' || Array.isArray(options)
      || Object.keys(options).some(key => !allowedOptions.includes(key))
      || (options.cwd !== undefined && (typeof options.cwd !== 'string' || !options.cwd.trim() || options.cwd.includes('\0')))
      || (options.allowNonZero !== undefined && typeof options.allowNonZero !== 'boolean')
      || Buffer.byteLength(JSON.stringify([program, args, options])) > LIMIT) {
    throw fault('invalid_input', '程序、参数或工作目录无效；参数必须是字符串数组。');
  }
}

function start(program, args = [], options = {}) {
  validateProcess(program, args, options, ['cwd', 'windowsHide']);
  const requestedWindowsHide = options.windowsHide;
  const windowsHide = requestedWindowsHide === undefined ? true : requestedWindowsHide;
  if (typeof windowsHide !== 'boolean') {
    throw fault('invalid_input', 'windowsHide 必须是布尔值。');
  }
  return new Promise((resolve, reject) => {
    const child = spawn(program, args, {
      cwd: options.cwd ?? request.package_root,
      shell: false, windowsHide, detached: true, stdio: 'ignore',
    });
    child.once('error', error => reject(fault('process_start_failed', error.message)));
    child.once('spawn', () => {
      child.unref();
      resolve({ pid: child.pid });
    });
  });
}

function run(program, args = [], options = {}) {
  validateProcess(program, args, options, ['cwd', 'allowNonZero']);
  return new Promise((resolve, reject) => {
    const child = spawn(program, args, {
      cwd: options.cwd ?? request.package_root,
      shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
    });
    children.add(child);
    let bytes = 0;
    let failure;
    const stdout = [];
    const stderr = [];
    const capture = (chunks, chunk) => {
      bytes += chunk.length;
      if (bytes > LIMIT) {
        failure ??= fault('output_limit', '子程序输出超过 64 KiB；程序已请求终止。');
        child.kill();
      } else chunks.push(chunk);
    };
    child.stdout.on('data', chunk => capture(stdout, chunk));
    child.stderr.on('data', chunk => capture(stderr, chunk));
    const aborted = () => {
      failure ??= controller.signal.reason;
      child.kill();
    };
    controller.signal.addEventListener('abort', aborted, { once: true });
    child.on('error', error => { failure ??= fault('process_start_failed', error.message); });
    child.on('close', (code, signal) => {
      children.delete(child);
      controller.signal.removeEventListener('abort', aborted);
      if (failure) return reject(failure);
      if (code !== 0 && !options.allowNonZero) {
        return reject(fault('process_failed', `子程序未正常退出（退出码 ${code ?? signal ?? '未知'}）。`));
      }
      resolve({ code, stdout: Buffer.concat(stdout).toString('utf8'), stderr: Buffer.concat(stderr).toString('utf8') });
    });
  });
}

function respond(result) {
  if (replied) return;
  replied = true;
  let response;
  const envelope = { api_version: 1, invocation_id: request.invocation.invocation_id };
  try {
    response = JSON.stringify({ ...envelope, ...result });
    if (Buffer.byteLength(response) > LIMIT) throw fault('output_limit', '动作结果超过 64 KiB。');
  } catch (error) {
    response = JSON.stringify({ ...envelope, status: 'error', error: {
      code: error.code === 'output_limit' ? 'output_limit' : 'invalid_output',
      message: error.code === 'output_limit' ? error.message : '动作结果必须可序列化为 JSON。',
    } });
  }
  writeFileSync(request.response_path + '.tmp', response, { flag: 'wx', mode: 0o600 });
  renameSync(request.response_path + '.tmp', request.response_path);
}

let timer;
try {
  await initial;
  if (request.api_version !== 1 || !request.invocation
      || typeof request.invocation.invocation_id !== 'string'
      || typeof request.invocation.binding_id !== 'string'
      || typeof request.entry_path !== 'string' || typeof request.package_root !== 'string'
      || typeof request.response_path !== 'string' || !request.input || typeof request.input !== 'object' || Array.isArray(request.input)) {
    throw fault('invalid_request', '不支持的动作宿主请求。');
  }
  timer = setTimeout(() => stop(fault('timeout', '动作超过运行时限；不会自动重试。')), request.timeout_ms);
  browserDeadline = Date.now() + request.timeout_ms;
  checkCancelled();
  const module = await import(pathToFileURL(request.entry_path).href);
  if (typeof module.default !== 'function') throw fault('invalid_entry', '动作入口必须 default 导出一个函数。');
  const ctx = Object.freeze({
    apiVersion: 1,
    input: request.input,
    invocation: Object.freeze(request.invocation),
    packageRoot: request.package_root,
    open: openTarget,
    openFileInParent,
    process: Object.freeze({ run, start, capture:createProcessCaptureApi(params => nativeCall('process.capture', params.cwd === undefined ? {...params,cwd:request.package_root} : params), true).capture }),
    state:createStateApi((method,params) => {
      if (request.actions_scope !== true) throw Object.assign(new Error('当前宿主没有动作状态作用域。'),{code:'state_unavailable',effectState:'not_started',method});
      return nativeCall(method,params);
    },true),
    actions:createActionsApi((method,params) => {
      if (request.actions_scope !== true) throw Object.assign(new Error('当前宿主没有ActionService调用作用域。'),{code:'actions_unavailable',effectState:'not_started',method});
      return nativeCall(method,params);
    },true),
    browser,
    desktop,
    clipboard,
    ui,
    fs,
    delay,
    signal: controller.signal,
  });
  const cancelled = new Promise((_, reject) => {
    if (controller.signal.aborted) reject(controller.signal.reason);
    else controller.signal.addEventListener('abort', () => reject(controller.signal.reason), { once: true });
  });
  const output = await Promise.race([module.default(ctx), cancelled]);
  checkCancelled();
  if (children.size) throw fault('pending_processes', '动作返回时仍有未等待的 ctx.process.run 调用；请使用 await。');
  if (browserPending.size) throw fault('pending_browser_calls', '动作返回时仍有未等待的浏览器调用；请使用 await。');
  if (nativePending) {
    if (nativePendingMethod === 'state.read' || nativePendingMethod === 'state.compare_and_swap') throw Object.assign(fault('pending_state_call', '动作返回时仍有未等待的状态调用；请使用 await。'), {effectState:nativePendingMethod === 'state.read' ? 'not_started' : 'unknown',method:nativePendingMethod});
    if (nativePendingMethod === 'system.open' || nativePendingMethod === 'system.open_file_in_parent') throw Object.assign(fault('pending_open', '动作返回时仍有未等待的打开请求；请使用 await。'), {effectState:'unknown',method:nativePendingMethod});
    if (nativePendingMethod === 'process.capture') throw Object.assign(fault('pending_process_capture', '动作返回时仍有未等待的程序采集；请使用 await。'), {effectState:'unknown',method:'process.capture'});
    if (nativePendingMethod === 'fs.stat' || nativePendingMethod === 'fs.read_directory') throw Object.assign(fault('fs_error', '动作返回时仍有未等待的文件查询；请使用 await。'), {effectState:'not_started',method:nativePendingMethod});
    throw Object.assign(fault('pending_desktop_calls', '动作返回时仍有未等待的桌面调用；请使用 await。'), {effectState:'unknown'});
  }
  respond({ status: 'ok', output: output ?? null });
} catch (error) {
  if (request?.response_path && request?.invocation) {
    respond({ status: 'error', error: {
      code: typeof error?.code === 'string' && /^[a-zA-Z0-9_]{1,64}$/.test(error.code) ? error.code : 'script_failed',
      message: String(error?.message ?? error).slice(0, 4096),
      ...(typeof error?.effectState === 'string' ? { effect_state: error.effectState } : {}),
      ...(typeof error?.method === 'string' ? { method: error.method } : {}),
      ...(Array.isArray(error?.requiredPermissions) ? { required_permissions: error.requiredPermissions } : {}),
    } });
  } else process.exitCode = 1;
} finally {
  clearTimeout(timer);
  stop(fault('cancelled', '动作宿主已结束。'));
  browserClient?.close();
  lines.close();
  // An action cannot keep this per-invocation host alive with stray timers/handles.
  process.exit(process.exitCode ?? 0);
}
