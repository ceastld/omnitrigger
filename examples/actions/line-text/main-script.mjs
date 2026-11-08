import {prepare,done,send,complete,sanitized} from './workflow.mjs';
/** @param {import('./line-types').SyncContext} ctx @returns {import('./line-types').LineReceipt} */
export function main(ctx){
  let method='script.parameters';
  try{
    const task=prepare(ctx.input);const value=done(task);
    if(value)return value;
    method='keyboard.text';
    return complete(task,send(ctx,task));
  }catch(error){throw sanitized(error,method);}
}
