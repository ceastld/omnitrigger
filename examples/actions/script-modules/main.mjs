import { cleanText } from './lib/text.js';

/** @type {ScriptMain} */
export function main(ctx) {
    return { text: cleanText(ctx.input.text), invocation: ctx.invocation };
}
