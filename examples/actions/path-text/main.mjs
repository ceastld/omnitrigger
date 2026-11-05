import {prepare,send,complete,sanitized} from './workflow.mjs';
export default async function main(ctx){
  let method='script.parameters';
  try{
    const task=prepare(ctx.input);
    if(task.mode==='inspect')return{status:'inspected',...task.summary,inputSubmitted:false};
    method='keyboard.text';
    return complete(task,await send(ctx,task));
  }catch(error){throw sanitized(error,method);}
}
