// Node 配方先做纯参数检查与快照，避免后面才发现的参数错误留下前面输入。
/** @param {string} message @returns {never} */
function invalid(message) {
  throw Object.assign(new Error(message), {
    code: 'invalid_input', effectState: 'not_started', method: 'script.parameters',
  });
}
/** @param {unknown} value @param {number} maximum @returns {value is number} */
const integer = (value, maximum) => typeof value === 'number' && Number.isInteger(value)
  && value >= 0 && value <= maximum;
/** @param {unknown} value @returns {value is number} */
const vk = value => integer(value, 254) && value > 0;

/** @typedef {{kind:'sequence',modifiers:ReadonlyArray<number>,keys:ReadonlyArray<number>} | {kind:'tap',key:number} | {kind:'text',text:string} | {kind:'delay',milliseconds:number}} RecipeStep */
/** @typedef {{steps:ReadonlyArray<RecipeStep>,requestedDelay?:number,hasSequence:boolean}} RecipePlan */
/** @param {unknown} input @returns {RecipePlan} */
export function prepareRecipe(input) {
  if (!input || typeof input !== 'object' || Array.isArray(input)) invalid('输入需要对象。');
  const value = /** @type {Record<string,any>} */ (input);
  const requestedDelay = value.keyDownDelayMs;
  const raw = value.steps;
  if (!Array.isArray(raw) || raw.length < 1 || raw.length > 32) invalid('需要 1–32 个输入步骤。');
  if (requestedDelay !== undefined && !integer(requestedDelay, 1000)) invalid('按下延迟需要 0–1000ms 整数。');
  /** @type {RecipeStep[]} */
  const steps = [];
  for (const step of raw) {
    if (!step || typeof step !== 'object' || Array.isArray(step)) invalid('输入步骤需要对象。');
    switch (step.kind) {
      case 'sequence': {
        if (!Array.isArray(step.modifiers) || step.modifiers.length > 4
            || !Array.isArray(step.keys) || step.keys.length < 1 || step.keys.length > 16
            || [...step.modifiers, ...step.keys].some(key => !vk(key))) invalid('连续按键的 VK 或数量无效。');
        if ((step.keys.length + 1) * (requestedDelay ?? 10) > 3000) invalid('单次按键等待合计不能超过 3000ms。');
        steps.push(Object.freeze({ kind: 'sequence', modifiers: Object.freeze([...step.modifiers]), keys: Object.freeze([...step.keys]) }));
        break;
      }
      case 'tap':
        if (!vk(step.key)) invalid('轻点按键需要明确 VK。');
        steps.push(Object.freeze({ kind: 'tap', key: step.key }));
        break;
      case 'text':
        if (typeof step.text !== 'string' || step.text.length > 65536 || step.text.includes('\0')) invalid('文本需要有效 Unicode，最多 65,536 个 UTF-16 单元。');
        for (const char of step.text) {
          if (char.length === 1 && char.charCodeAt(0) >= 0xd800 && char.charCodeAt(0) <= 0xdfff) invalid('文本包含孤立代理项。');
        }
        steps.push(Object.freeze({ kind: 'text', text: step.text }));
        break;
      case 'delay':
        if (!integer(step.milliseconds, 60000)) invalid('等待需要 0–60000ms 整数。');
        steps.push(Object.freeze({ kind: 'delay', milliseconds: step.milliseconds }));
        break;
      default: invalid('输入步骤类型无效。');
    }
  }
  const hasSequence = steps.some(step => step.kind === 'sequence');
  if (steps.length + Number(hasSequence && requestedDelay === undefined) > 32) {
    invalid('步骤与目标查询合计不能超过 32 次宿主调用；可显式指定按下延迟。');
  }
  return Object.freeze({ steps: Object.freeze(steps), requestedDelay, hasSequence });
}
