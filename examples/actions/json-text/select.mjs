import {prepareSelection,selectionReceipt,selectionError} from './select-core.mjs';
/** @param {import('./json-types').NodeContext} ctx @returns {Promise<import('./json-types').SelectionActionResult>} */
export default async function main(ctx) {
  let attempted=false;
  try {
    const task=prepareSelection(ctx.input);
    if(task.mode==='inspect')return {status:'inspected',...task.summary,inputSubmitted:false};
    if(typeof ctx.desktop?.sendText!=='function')throw Object.assign(new Error(''),{code:'desktop_unavailable'});
    attempted=true;
    return selectionReceipt(task,await ctx.desktop.sendText(task.text));
  } catch(error) {throw selectionError(error,attempted);}
}
