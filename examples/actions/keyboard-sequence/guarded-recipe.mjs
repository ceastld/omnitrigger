/// <reference path="./sdk.d.ts" />
import {prepareRecipe} from './prepare-recipe.mjs';

/** 仅作作者类型提示；运行时仍执行名单、步骤和预算校验。
 * @typedef {{allowedProcesses:ReadonlyArray<string>,steps:ReadonlyArray<import('./prepare-recipe.mjs').RecipeStep>,keyDownDelayMs?:number}} GuardedRecipeInput
 */

/** @param {string} message @returns {never} */
function invalid(message) {
  throw Object.assign(new Error(message), {
    code:'invalid_input', effectState:'not_started', method:'script.parameters',
  });
}

/** @param {unknown} input */
function requiredProcesses(input) {
  const names = input && typeof input === 'object' && !Array.isArray(input)
    ? /** @type {Record<string,unknown>} */ (input).allowedProcesses : undefined;
  if (!Array.isArray(names) || names.length < 1 || names.length > 16
      || names.some(name => typeof name !== 'string' || !/^[A-Za-z0-9_.-]{1,64}$/.test(name)
        || name === '.' || name === '..' || /\.exe$/i.test(name))
      || new Set(names.map(name => name.toLowerCase())).size !== names.length)
    invalid('必须提供1–16个不重复的allowedProcesses基本名，不含路径或.exe。');
  return Object.freeze(names.map(name => /** @type {string} */ (name).toLowerCase()));
}

/** @type {ScriptMain<GuardedRecipeInput>} */
export function main(ctx) {
  // 在查询原目标和发送输入前检查完整名单与步骤。
  const allowed = requiredProcesses(ctx.input);
  const plan = prepareRecipe(ctx.input);
  // 即使显式给出按下延时，进程守卫仍须查询一次原目标。
  if (plan.steps.length + 1 > 32) invalid('步骤与必需原目标查询合计不能超过32次宿主调用。');
  const processName = ctx.desktop.target().processName.toLowerCase().replace(/\.exe$/, '');
  if (!allowed.includes(processName))
    throw Object.assign(new Error('原目标进程不在此动作明确允许的范围内。'), {
      code:'target_process_not_allowed', effectState:'not_started', method:'desktop.target',
    });
  const keyDownDelayMs = plan.requestedDelay ?? (['wps','et','wpp','wpspdf','mstsc'].includes(processName) ? 10 : 0);
  for (const step of plan.steps) {
    switch (step.kind) {
      case 'sequence': ctx.desktop.sendKeySequence([...step.modifiers], [...step.keys], {keyDownDelayMs}); break;
      case 'tap': ctx.desktop.sendKeySequence([], [step.key], {keyDownDelayMs:0}); break;
      case 'text': ctx.desktop.sendText(step.text); break;
      case 'delay': ctx.delay(step.milliseconds); break;
    }
  }
  return {inputSubmitted:plan.steps.some(step => step.kind !== 'delay'), steps:plan.steps.length};
}
