import {cleanText} from '../shared/text.mjs';

/** @type {ScriptMain} */
export function main(ctx) {
  const target = ctx.desktop.target();
  return {text:cleanText(ctx.input.text),processName:target.processName};
}
