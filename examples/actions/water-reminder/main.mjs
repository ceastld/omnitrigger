import {prepare,available,complete,sanitized} from './notice.mjs';
/** @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx){
  let method='script.parameters',began=false,returned=false;
  try{
    const options=prepare(ctx.input);method='ui.notify';available(ctx);ctx.signal.throwIfAborted();
    began=true;const receipt=await ctx.ui.notify(options);returned=true;
    ctx.signal.throwIfAborted();return complete(receipt);
  }catch(error){throw sanitized(error,method,began,returned);}
}
