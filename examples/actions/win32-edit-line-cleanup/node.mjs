import {configurationFromInput,prepare,confirmOriginal,processed,sendFailure,checkCancelled} from './lib/selection.mjs';
/** @param {import('./types/api.d.ts').ActionContext} ctx */
export default async function main(ctx) {
  const {operation,adapter}=configurationFromInput(ctx.input);checkCancelled(ctx.signal);
  const plan=prepare(await ctx.desktop.readSelectionText({maxUtf16Units:8192,overflow:'reject'}),operation);
  checkCancelled(ctx.signal);
  if(plan.kind!=='ready') return {status:plan.kind};
  confirmOriginal(plan,await ctx.desktop.readSelectionText({maxUtf16Units:8192,overflow:'reject'}));
  checkCancelled(ctx.signal);
  try {return processed(await ctx.desktop.win32EditReplaceSelection(plan.replacement,{adapter}));}
  catch(error){throw sendFailure(error);}
}
