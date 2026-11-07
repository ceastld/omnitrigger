/// <reference path="./sdk.d.ts" />
import {validateFoldInput, foldIncomplete} from './fold-all-support.mjs';

/** @type {ScriptMain<Record<string,never>>} */
export function main(ctx) {
  validateFoldInput(ctx.input);
  let accepted = 0;
  /** @type {import('./fold-all-support.mjs').FoldPhase} */
  let phase = 'first_shortcut';
  try {
    const first = ctx.desktop.sendShortcut([162,77], {keyDownDelayMs:0});
    if (first?.submitted !== true) throw Object.assign(new Error('首段按键缺少确认。'), {code:'submission_unknown',effectState:'unknown'});
    accepted++;
    phase = 'inter_chord_delay';
    ctx.delay(50);
    phase = 'second_shortcut';
    const second = ctx.desktop.sendShortcut([162,76], {keyDownDelayMs:0});
    if (second?.submitted !== true) throw Object.assign(new Error('末段按键缺少确认。'), {code:'submission_unknown',effectState:'unknown'});
    accepted++;
    return {inputSubmitted:true,acceptedSubmissions:accepted};
  } catch (cause) {
    foldIncomplete(cause, accepted, phase);
  }
}
