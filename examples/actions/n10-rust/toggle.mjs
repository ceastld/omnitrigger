import { win32 as path } from 'node:path';

function localPath(value) {
  return typeof value === 'string' && /^[a-z]:[\\/]/i.test(value) &&
    !value.includes('\0') && !value.split(/[\\/]/).includes('..');
}

function samePath(actual, expected) {
  if (typeof actual !== 'string') return false;
  const normalize = value => path.resolve(value.replace(/^\\\\\?\\/, '')).toLowerCase();
  return normalize(actual) === normalize(expected);
}

function fault(message, effectState) {
  return Object.assign(new Error(message), { code: 'clipboard_toggle_failed', effectState });
}

async function runToggle(ctx, target, testProfile) {
  const { cliExecutable, hostExecutable, dataRoot } = ctx.input;
  // 给不同进程的时钟留出余量，且只缩短宿主签发的有效期。
  const expires = Math.min(target.expires_at_unix_ms, Date.now() + 20_000);
  const args = ['toggle', '--host', hostExecutable, testProfile ? '--test-data-root' : '--expected-data-root', dataRoot,
    '--target-context', JSON.stringify({ ...target, expires_at_unix_ms: expires }),
    '--timeout-ms', '15000', '--json'];
  // run 允许按需启动的独立剪贴板在本次调用结束后继续采集。
  const { code, stdout } = await ctx.process.run(cliExecutable, args, { allowNonZero: true });
  try {
    return { code, reply: JSON.parse(stdout) };
  } catch {
    throw fault('未能读取剪贴板回执。', 'unknown');
  }
}

export default async function toggle(ctx) {
  const { cliExecutable, hostExecutable, dataRoot, profile = 'product' } = ctx.input ?? {};
  if (![cliExecutable, hostExecutable, dataRoot].every(localPath)) {
    throw fault('剪贴板客户端、宿主和数据目录需要完整本机路径。', 'not_started');
  }
  if (!['product', 'script-test'].includes(profile)) {
    throw fault('剪贴板实例只支持 product 或 script-test。', 'not_started');
  }
  const testProfile = profile === 'script-test';
  const original = await ctx.desktop.targetContext();
  let { code, reply } = await runToggle(ctx, original, testProfile);
  if (code !== 0 && reply?.ok === false && reply.state === 'failed' && reply.submitted === false &&
      ['invalid_arguments', 'local_error'].includes(reply.error?.code) &&
      reply.error.message === 'invalid_or_expired_target') {
    // targetContext 会重新核验本次按键捕获的窗口，再签发短时快照。
    const fresh = await ctx.desktop.targetContext();
    if (!['window_handle', 'process_id', 'thread_id', 'process_created_file_time']
      .every(field => original[field] === fresh[field])) {
      throw fault('原目标窗口已改变，请在需要粘贴的窗口重新按热键。', 'not_started');
    }
    ({ code, reply } = await runToggle(ctx, fresh, testProfile));
  }
  if (code !== 0 || reply?.ok !== true) {
    throw fault(reply?.error?.message || '剪贴板未确认切换。',
      reply?.submitted === false ? 'not_started' : 'unknown');
  }
  const result = reply.result;
  if (reply.state !== 'accepted' || result?.targetContextVerified !== true ||
      result.targetContextVersion !== 1 || result.testProfile !== testProfile ||
      (testProfile && result.captureEnabled !== false) ||
      !samePath(result.executable, hostExecutable) || !samePath(result.dataRoot, dataRoot)) {
    throw fault('剪贴板回执与当前数据目录或目标窗口不一致。', 'unknown');
  }
  return { accepted: true, started: result.started };
}
