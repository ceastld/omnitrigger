const method = 'clipboard.write_files';
/** @param {string} code @param {string} message @param {'not_started'|'unknown'} [effectState] */
function fail(code, message, effectState = 'not_started') {
  return Object.assign(new Error(message), { code, effectState, method });
}

/** @param {Pick<import('./sdk/api.js').ActionContext,'clipboard'> | Pick<ScriptContext<Record<string,unknown>>,'clipboard'>} ctx */
export function requireWriter(ctx) {
  if (typeof ctx.clipboard?.writeFiles !== 'function') {
    throw fail('api_unavailable', '此动作需要文件列表剪贴板写入 API。');
  }
}

/** @param {import('./sdk/api.js').ExplorerContextSnapshot | ScriptExplorerContextSnapshot} snapshot
 * @returns {readonly string[]} */
export function selectedPaths(snapshot) {
  const paths = snapshot?.selectedPaths;
  if (!Array.isArray(paths) || paths.length < 1 || paths.length > 30
      || Array.from(paths).some(/** @param {unknown} path */ path => typeof path !== 'string' || !path.length)) {
    throw fail('invalid_input', '请在原资源管理器窗口中选择 1–30 个文件或文件夹。');
  }
  return Object.freeze([...paths]);
}

/** @param {import('./sdk/api.js').LocalPathStat | ScriptLocalPathStat} stat */
export function requireFileOrDirectory(stat) {
  if (!stat || !['file', 'directory'].includes(stat.kind)) {
    throw fail('file_required', '所选项目必须全部是可用的本地文件或文件夹。');
  }
}

/** @param {Pick<import('./sdk/api.js').ActionContext,'signal'>} ctx @param {boolean} [afterWrite] */
export function checkCancelled(ctx, afterWrite = false) {
  if (ctx.signal?.aborted) {
    throw fail('cancelled', afterWrite ? '写入后观察到取消；文件列表效果未确认。' : '动作在写入前取消。', afterWrite ? 'unknown' : 'not_started');
  }
}
