/// <reference path="./sdk/script-types.d.ts" />
import {prepare,available,execute,notification,complete,sanitized} from './workflow.mjs';

/** @type {ScriptMain<Record<string, unknown>>} */
export function main(ctx) {
  let method='script.parameters';let completed=false;
  try{
    const task=prepare(ctx.input,true);
    available(ctx,task);
    method='process.capture';
    const result=execute(ctx,task);completed=true;
    const value=notification(result,task);
    method='ui.notify';
    return complete(value,ctx.ui.notify(value.params));
  }catch(error){throw sanitized(error,method,completed);}
}
