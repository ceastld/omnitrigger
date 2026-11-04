/// <reference path="./sdk/api.d.ts" />
/** @param {ScriptContext} ctx */
function main(ctx) {
    const text = String(ctx.input.text ?? '').trim();
    return { text, invocation: ctx.invocation };
}
