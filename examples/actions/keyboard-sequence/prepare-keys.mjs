// 只预检两个现有入口已公开的字段；最终平台语义仍由同一次桌面 API 核验。
/** @param {string} method @param {string} message @returns {never} */
function invalid(method, message) {
  throw Object.assign(new Error(message), { code:'invalid_input', effectState:'not_started', method });
}
/** @param {unknown} raw @returns {Record<string,any>} */
function object(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) invalid('script.parameters', '输入需要对象。');
  return /** @type {Record<string,any>} */ (raw);
}
/** @param {unknown} value @returns {value is number} */
const vk = value => Number.isInteger(value) && typeof value === 'number' && value >= 1 && value <= 254;
/** @param {unknown} value @param {string} method @returns {number|undefined} */
function requestedDelay(value, method) {
  if (value !== undefined && (typeof value !== 'number' || !Number.isInteger(value) || value < 0 || value > 1000)) {
    invalid(method, '按下延迟需要 0–1000ms 整数。');
  }
  return value;
}

/** @param {unknown} raw */
export function prepareSequence(raw) {
  const input = object(raw);
  const method = 'keyboard.sequence';
  const { modifiers, keys } = input;
  const keyDownDelayMs = requestedDelay(input.keyDownDelayMs, method);
  if (!Array.isArray(modifiers) || modifiers.length > 4 || !Array.isArray(keys) || keys.length < 1 || keys.length > 16
      || [...modifiers, ...keys].some(key => !vk(key))) {
    invalid(method, '连续按键需要 0–4 个修饰键和 1–16 个普通键，使用明确 VK 整数。');
  }
  // 已有自动进程延迟只有 0/10；显式延迟按公开 API 的 count+1 预算核对。
  if ((keys.length + 1) * (keyDownDelayMs ?? 10) > 3000) invalid(method, '单次按键等待合计不能超过 3000ms。');
  return Object.freeze({ modifiers:Object.freeze(/** @type {number[]} */ (modifiers.slice())),
    keys:Object.freeze(/** @type {number[]} */ (keys.slice())), keyDownDelayMs });
}

/** @typedef {{keys:ReadonlyArray<number>,delayAfterMs?:number}} ShortcutStep */
/** @param {unknown} raw */
export function prepareShortcuts(raw) {
  const input = object(raw);
  const method = 'keyboard.shortcut';
  const keyDownDelayMs = requestedDelay(input.keyDownDelayMs, method);
  if (!Array.isArray(input.steps) || input.steps.length < 1 || input.steps.length > 16) {
    invalid('script.parameters', '需要 1–16 个快捷键步骤。');
  }
  /** @type {ShortcutStep[]} */
  const steps = [];
  for (const step of input.steps) {
    if (!step || typeof step !== 'object' || Array.isArray(step) || !Array.isArray(step.keys)
        || step.keys.length < 1 || step.keys.length > 5 || step.keys.some(/** @param {unknown} key */ key => !vk(key))) {
      invalid(method, '快捷键需要 1–5 个明确的 VK 整数；修饰键在前。');
    }
    const delayAfterMs = step.delayAfterMs;
    if (delayAfterMs !== undefined && (!Number.isInteger(delayAfterMs) || delayAfterMs < 0 || delayAfterMs > 60000)) {
      invalid('script.parameters', '等待时间必须为 0–60000 毫秒的整数。');
    }
    steps.push(Object.freeze({ keys:Object.freeze(/** @type {number[]} */ (step.keys.slice())),
      ...(delayAfterMs !== undefined ? { delayAfterMs } : {}) }));
  }
  return Object.freeze({ steps:Object.freeze(steps), keyDownDelayMs });
}
