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

export default async function toggle(ctx) {
  const { cliExecutable, hostExecutable, dataRoot } = ctx.input ?? {};
  if (![cliExecutable, hostExecutable, dataRoot].every(localPath)) {
    throw fault('剪贴板客户端、宿主和数据目录需要完整本机路径。', 'not_started');
  }
  const target = await ctx.desktop.targetContext();
  const args = ['toggle', '--host', hostExecutable, '--expected-data-root', dataRoot,
    '--target-context', JSON.stringify(target), '--timeout-ms', '15000', '--json'];
  // run 允许按需启动的独立剪贴板在本次调用结束后继续采集。
  const { code, stdout } = await ctx.process.run(cliExecutable, args, { allowNonZero: true });
  let reply;
  try {
    reply = JSON.parse(stdout);
  } catch {
    throw fault('未能读取剪贴板回执。', 'unknown');
  }
  if (code !== 0 || reply?.ok !== true) {
    throw fault(reply?.error?.message || '剪贴板未确认切换。',
      reply?.submitted === false ? 'not_started' : 'unknown');
  }
  const result = reply.result;
  if (reply.state !== 'accepted' || result?.targetContextVerified !== true ||
      result.targetContextVersion !== 1 || result.testProfile !== false ||
      !samePath(result.executable, hostExecutable) || !samePath(result.dataRoot, dataRoot)) {
    throw fault('剪贴板回执与当前数据目录或目标窗口不一致。', 'unknown');
  }
  return { accepted: true, started: result.started };
}
