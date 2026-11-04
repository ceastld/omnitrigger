/// <reference path="../../../tools/js-engine-probe/api.d.ts" />
/** @type {ScriptMain} */
function main(ctx) {
    const operation = ctx.input.operation;
    if (!['toggle_maximize', 'minimize', 'close'].includes(operation)) {
        throw Object.assign(new Error('operation 需要 toggle_maximize、minimize 或 close。'), {
            code: 'invalid_input', effectState: 'not_started', method: 'script.parameters',
        });
    }
    const receipt = ctx.desktop.windowAction(operation);
    return {operation, submitted: receipt.submitted};
}
