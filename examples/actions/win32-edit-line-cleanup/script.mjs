/// <reference path="./sdk/api.d.ts" />
import {configurationFromInput,prepare,confirmOriginal,processed,sendFailure} from './lib/selection.mjs';
/** @param {ScriptContext} ctx */
export default function main(ctx) {
  const {operation,adapter}=configurationFromInput(ctx.input);
  const plan=prepare(ctx.desktop.readSelectionText({maxUtf16Units:8192,overflow:'reject'}),operation);
  if(plan.kind!=='ready') return {status:plan.kind};
  confirmOriginal(plan,ctx.desktop.readSelectionText({maxUtf16Units:8192,overflow:'reject'}));
  // 同步取消由现有宿主/NativeHost处理；不增加脚本取消API。
  try {return processed(ctx.desktop.win32EditReplaceSelection(plan.replacement,{adapter}));}
  catch(error){throw sendFailure(error);}
}

export {main};
