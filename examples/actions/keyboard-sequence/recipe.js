/// <reference path="./sdk.d.ts" />
/** @type {ScriptMain} */
function main(ctx) {
    /** @param {string} message @returns {never} */
    function invalid(message) {
        throw Object.assign(new Error(message), {
            code: 'invalid_input', effectState: 'not_started', method: 'script.parameters',
        });
    }
    /** @param {unknown} value @param {number} maximum */
    function integer(value, maximum) {
        return typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= maximum;
    }
    /** @typedef {{kind:'sequence',modifiers:number[],keys:number[]} | {kind:'tap',key:number} | {kind:'text',text:string} | {kind:'delay',milliseconds:number}} Step */
    /** @type {Step[]} */
    const steps = ctx.input.steps;
    const requestedDelay = ctx.input.keyDownDelayMs;
    if (!Array.isArray(steps) || steps.length < 1 || steps.length > 32) invalid('需要 1–32 个输入步骤。');
    if (requestedDelay !== undefined && !integer(requestedDelay, 1000)) invalid('按下延迟需要 0–1000ms 整数。');
    // 先检查所有步骤的形状、等待和宿主调用预算，避免这些参数错误留下前面步骤的输入。
    for (const step of steps) {
        if (!step || typeof step !== 'object') invalid('输入步骤需要对象。');
        switch (step.kind) {
            case 'sequence':
                if (!Array.isArray(step.modifiers) || step.modifiers.length > 4
                    || !Array.isArray(step.keys) || step.keys.length < 1 || step.keys.length > 16
                    || [...step.modifiers, ...step.keys].some(key => !integer(key, 254) || key === 0)) invalid('连续按键的 VK 或数量无效。');
                if ((step.keys.length + 1) * (requestedDelay ?? 10) > 3000) invalid('单次按键等待合计不能超过 3000ms。');
                break;
            case 'tap': if (!integer(step.key, 254) || step.key === 0) invalid('轻点按键需要明确 VK。'); break;
            case 'text':
                if (typeof step.text !== 'string' || step.text.length > 65536 || step.text.includes('\0')) invalid('文本需要不含 NUL 的有效 Unicode，最多 65,536 个 UTF-16 单元。');
                for (const character of step.text) {
                    if (character.length === 1 && character.charCodeAt(0) >= 0xd800 && character.charCodeAt(0) <= 0xdfff) invalid('文本包含孤立代理项。');
                }
                break;
            case 'delay': if (!integer(step.milliseconds, 60000)) invalid('等待需要 0–60000ms 整数。'); break;
            default: invalid('输入步骤类型无效。');
        }
    }
    const queryTarget = steps.some(step => step.kind === 'sequence') && requestedDelay === undefined;
    if (steps.length + Number(queryTarget) > 32) invalid('步骤与目标查询合计不能超过 32 次宿主调用；可显式指定按下延迟。');
    const processName = queryTarget ? ctx.desktop.target().processName.toLowerCase().replace(/\.exe$/, '') : '';
    const keyDownDelayMs = requestedDelay ?? (['wps', 'et', 'wpp', 'wpspdf', 'mstsc'].includes(processName) ? 10 : 0);
    for (const step of steps) {
        switch (step.kind) {
            case 'sequence': ctx.desktop.sendKeySequence(step.modifiers, step.keys, {keyDownDelayMs}); break;
            case 'tap': ctx.desktop.sendKeySequence([], [step.key], {keyDownDelayMs:0}); break;
            case 'text': ctx.desktop.sendText(step.text); break;
            case 'delay': ctx.delay(step.milliseconds); break;
        }
    }
    return {inputSubmitted: steps.some(step => step.kind !== 'delay'), steps: steps.length};
}
