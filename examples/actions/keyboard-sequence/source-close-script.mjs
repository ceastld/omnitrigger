/// <reference path="./sdk.d.ts" />
// 固定关闭配方：按本次原目标进程分支，发送一次来源快捷键。
/** @param {string} code @param {string} message @param {string} method @param {'not_started'|'unknown'} [effectState] @returns {never} */
function fail(code, message, method, effectState = 'not_started') {
  throw Object.assign(new Error(message), {code, method, effectState});
}
/** @type {ScriptMain<Record<string,never>>} */
export function main(ctx) {
  if (!ctx.input || typeof ctx.input !== 'object' || Array.isArray(ctx.input)
      || Reflect.ownKeys(ctx.input).length !== 0) {
    fail('invalid_input', '此固定关闭配方不接受输入参数。', 'script.parameters');
  }
  const target = ctx.desktop.target();
  const name = target?.processName;
  if (!target || typeof name !== 'string' || name.length > 260 || name !== name.trim()
      || !/^[^\\/:*?"<>|\u0000-\u001f\u007f]+\.exe$/i.test(name)
      || !Number.isInteger(target.processId) || target.processId < 1 || target.processId > 0xffffffff) {
    fail('target_process_unknown', '无法确认原目标进程身份，未发送关闭快捷键。', 'target.info');
  }
  if (target.isForeground !== true || target.windowRole !== 'application') {
    fail('target_changed', '原目标不是当前前台应用窗口，未发送关闭快捷键。', 'target.info');
  }
  // 原条件使用大小写敏感的Contains；只去掉API返回文件名的.exe后缀。
  const process = name.slice(0, -4);
  if (!process) fail('target_process_unknown', '原目标进程名称为空。', 'target.info');
  const branch = process === 'Acrobat' ? 'acrobat-ctrl-w'
    : process === 'chrome' ? 'chrome-alt-f4' : 'other-alt-f4';
  const result = ctx.desktop.sendShortcut(branch === 'acrobat-ctrl-w' ? [162,87] : [164,115]);
  if (!result || result.submitted !== true || typeof /** @type {any} */ (result).then === 'function') {
    fail('input_unconfirmed', '关闭快捷键提交未获确认；不能据此重试。', 'keyboard.shortcut', 'unknown');
  }
  return {inputSubmitted:true, branch};
}
