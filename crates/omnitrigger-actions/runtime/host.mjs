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
let nativePending;
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

function chromeProxy(parts = []) {
  return new Proxy(function () {}, {
    get(_target, name) {
      if (name === 'then' || name === 'toJSON' || typeof name !== 'string') return undefined;
      return chromeProxy([...parts, name]);
    },
    apply(_target, _this, args) { return browserRequest(parts.join('.'), args); },
  });
}

const browser = Object.freeze({
  profiles: (options = {}) => browserRequest(undefined, undefined, options, true),
  call: (method, args = [], options = {}) => browserRequest(method, args, options),
  chrome: chromeProxy(),
});

function nativeCall(method, params) {
  checkCancelled();
  const fail = (code, message, effectState = 'not_started') => Object.assign(fault(code, message), {effectState, method});
  if (!request.native) return Promise.reject(fail(request.native_error?.code ?? 'desktop_unavailable', request.native_error?.message ?? '此宿主没有提供本次调用的桌面上下文。'));
  if (nativePending) return Promise.reject(fail('desktop_busy', '请 await 上一个桌面调用，避免输入并发。'));
  if (nativeId >= 128) return Promise.reject(fail('desktop_call_limit', '单次动作最多调用 128 次桌面 API。'));
  const id = ++nativeId;
  const operation = new Promise((resolve, reject) => {
    let settled = false;
    let sent = false;
    let chunks = '';
    const socket = createConnection({host:'127.0.0.1', port:request.native.port});
    nativeSockets.add(socket);
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      controller.signal.removeEventListener('abort', aborted);
      nativeSockets.delete(socket);
      socket.destroy();
      if (error) reject(error); else resolve(value);
    };
    const aborted = () => finish(controller.signal.reason);
    controller.signal.addEventListener('abort', aborted, {once:true});
    socket.setEncoding('utf8');
    socket.setTimeout(5000, () => finish(fail('desktop_timeout', '桌面调用未在 5 秒内确认；不会自动重试。', sent ? 'unknown' : 'not_started')));
    socket.once('connect', () => {
      if (controller.signal.aborted) return aborted();
      sent = true;
      socket.write(JSON.stringify({token:request.native.token, id, method, params}) + '\n');
    });
    socket.on('data', chunk => {
      chunks += chunk;
      if (Buffer.byteLength(chunks) > 512 * 1024 + 1) return finish(fail('desktop_protocol', '桌面回执超过上限。', 'unknown'));
      if (!chunks.endsWith('\n')) return;
      try {
        const response = JSON.parse(chunks);
        if (response.id !== id || typeof response.ok !== 'boolean') throw new Error('回执身份不匹配');
        if (response.ok) finish(undefined, response.value);
        else {
          const error = response.error;
          if (!error || typeof error.code !== 'string' || typeof error.message !== 'string') throw new Error('缺少错误');
          finish(fail(error.code, error.message, error.effect_state ?? 'unknown'));
        }
      } catch { finish(fail('desktop_protocol', '桌面回执格式无效；业务结果未确认。', 'unknown')); }
    });
    socket.once('error', error => finish(fail('desktop_connection', error.message, sent ? 'unknown' : 'not_started')));
    socket.once('close', () => finish(fail('desktop_connection', '桌面连接已关闭；不会重试。', sent ? 'unknown' : 'not_started')));
  });
  nativePending = operation;
  operation.then(() => { nativePending = undefined; }, () => { nativePending = undefined; });
  return operation;
}

const desktop = Object.freeze({
  target: () => nativeCall('target.info', {}),
  sendShortcut: (keys) => {
    if (!Array.isArray(keys) || keys.length < 1 || keys.length > 9
        || keys.some(key => !Number.isInteger(key) || key < 1 || key > 254)) {
      return Promise.reject(Object.assign(fault('invalid_input', '快捷键需要 1–9 个明确的 VK 整数；修饰键在前。'), {effectState:'not_started'}));
    }
    return nativeCall('keyboard.shortcut', {keys});
  },
  sendText: text => textCall('keyboard.text', text),
  copySelectionText: () => nativeCall('selection.copy', {}),
});

function textCall(method, text) {
  if (typeof text !== 'string' || text.length > 65536 || text.includes('\0')
      || Buffer.from(text, 'utf8').toString('utf8') !== text) {
    return Promise.reject(Object.assign(fault('invalid_input', '文本需要有效 Unicode，不含 NUL，最多 65,536 个 UTF-16 单元。'), {effectState:'not_started', method}));
  }
  return nativeCall(method, {text});
}

const clipboard = Object.freeze({
  readText: () => nativeCall('clipboard.read', {}),
  writeText: text => textCall('clipboard.write', text),
});

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
      || args.some(arg => typeof arg !== 'string' || arg.includes('\0'))
      || !options || typeof options !== 'object' || Array.isArray(options)
      || Object.keys(options).some(key => !allowedOptions.includes(key))
      || (options.cwd !== undefined && (typeof options.cwd !== 'string' || !options.cwd.trim() || options.cwd.includes('\0')))
      || (options.allowNonZero !== undefined && typeof options.allowNonZero !== 'boolean')
      || Buffer.byteLength(JSON.stringify([program, args, options])) > LIMIT) {
    throw fault('invalid_input', '程序、参数或工作目录无效；参数必须是字符串数组。');
  }
}

function start(program, args = [], options = {}) {
  validateProcess(program, args, options, ['cwd']);
  return new Promise((resolve, reject) => {
    const child = spawn(program, args, {
      cwd: options.cwd ?? request.package_root,
      shell: false, windowsHide: true, detached: true, stdio: 'ignore',
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
    process: Object.freeze({ run, start }),
    browser,
    desktop,
    clipboard,
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
  if (nativePending) throw Object.assign(fault('pending_desktop_calls', '动作返回时仍有未等待的桌面调用；请使用 await。'), {effectState:'unknown'});
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
