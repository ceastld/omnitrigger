/// <reference path="../../../tools/js-engine-probe/api.d.ts" />
/** @param {ScriptContext} ctx */
function main(ctx) {
    const target = ctx.desktop.target();
    const snapshot = ctx.desktop.targetContext();
    return { target, snapshot, invocation: ctx.invocation };
}
