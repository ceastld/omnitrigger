import {prepare,available,execute,notification,complete,sanitized} from './workflow.mjs';

/** @param {import('./sdk/api.js').ActionContext} ctx */
export default async function main(ctx) {
  let method='script.parameters';let completed=false;
  try{
    const task=prepare(ctx.input,true);
    available(ctx,task);
    method='process.capture';
    const result=await execute(ctx,task);completed=true;
    const value=notification(result,task);
    method='ui.notify';
    return complete(value,await ctx.ui.notify(value.params));
  }catch(error){throw sanitized(error,method,completed);}
}
