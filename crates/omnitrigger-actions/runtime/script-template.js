/// <reference path="./sdk/api.d.ts" />
/** @type {ScriptMain} */
function main(ctx) {
    const text = String(ctx.input.text ?? '').trim();
    return { text, invocation: ctx.invocation };
}
