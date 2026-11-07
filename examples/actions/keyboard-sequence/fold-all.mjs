/// <reference path="./sdk.d.ts" />
import {validateFoldInput, foldIncomplete} from './fold-all-support.mjs';

/**
 * @param {OmniTriggerActionContext} ctx
 * @returns {Promise<{inputSubmitted:true,acceptedSubmissions:number}>}
 */
export default async function foldAll(ctx) {
  validateFoldInput(ctx.input);
  let accepted = 0;
  /** @type {import('./fold-all-support.mjs').FoldPhase} */
  let phase = 'first_shortcut';
  try {
    const first = await ctx.desktop.sendShortcut([162,77], {keyDownDelayMs:0});
    if (first?.submitted !== true) throw Object.assign(new Error('首段按键缺少确认。'), {code:'submission_unknown',effectState:'unknown'});
    accepted++;
    phase = 'inter_chord_delay';
    await ctx.delay(50);
    phase = 'second_shortcut';
    const second = await ctx.desktop.sendShortcut([162,76], {keyDownDelayMs:0});
    if (second?.submitted !== true) throw Object.assign(new Error('末段按键缺少确认。'), {code:'submission_unknown',effectState:'unknown'});
    accepted++;
    return {inputSubmitted:true,acceptedSubmissions:accepted};
  } catch (cause) {
    foldIncomplete(cause, accepted, phase);
  }
}
