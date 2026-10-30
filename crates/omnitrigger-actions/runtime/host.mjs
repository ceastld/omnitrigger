// OmniTrigger JavaScript host API v1. Trusted local code; this is not a sandbox.
import { spawn } from 'node:child_process';
import { writeFileSync, renameSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';

const LIMIT = 64 * 1024;
const controller = new AbortController();
const children = new Set();
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
let request;
let replied = false;
const fault = (code, message) => Object.assign(new Error(message), { code });
const stop = (reason) => {
  if (!controller.signal.aborted) controller.abort(reason);
  for (const child of children) child.kill();
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
  checkCancelled();
  const module = await import(pathToFileURL(request.entry_path).href);
  if (typeof module.default !== 'function') throw fault('invalid_entry', '动作入口必须 default 导出一个函数。');
  const ctx = Object.freeze({
    apiVersion: 1,
    input: request.input,
    invocation: Object.freeze(request.invocation),
    packageRoot: request.package_root,
    process: Object.freeze({ run, start }),
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
  respond({ status: 'ok', output: output ?? null });
} catch (error) {
  if (request?.response_path && request?.invocation) {
    respond({ status: 'error', error: {
      code: typeof error?.code === 'string' && /^[a-zA-Z0-9_]{1,64}$/.test(error.code) ? error.code : 'script_failed',
      message: String(error?.message ?? error).slice(0, 4096),
    } });
  } else process.exitCode = 1;
} finally {
  clearTimeout(timer);
  stop(fault('cancelled', '动作宿主已结束。'));
  lines.close();
  // An action cannot keep this per-invocation host alive with stray timers/handles.
  process.exit(process.exitCode ?? 0);
}
