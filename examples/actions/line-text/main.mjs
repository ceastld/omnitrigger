import {prepare,done,send,complete,sanitized} from './workflow.mjs';
export default async function main(ctx){
  let method='script.parameters';
  try{
    const task=prepare(ctx.input);const value=done(task);
    if(value)return value;
    method='keyboard.text';
    return complete(task,await send(ctx,task));
  }catch(error){throw sanitized(error,method);}
}
