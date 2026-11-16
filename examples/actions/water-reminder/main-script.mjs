import {prepare,available,complete,sanitized} from './notice.mjs';
/** @type {ScriptMain} */
export function main(ctx){
  let method='script.parameters',began=false;
  try{
    const options=prepare(ctx.input);method='ui.notify';available(ctx);
    began=true;return complete(ctx.ui.notify(options));
  }catch(error){throw sanitized(error,method,began);}
}
