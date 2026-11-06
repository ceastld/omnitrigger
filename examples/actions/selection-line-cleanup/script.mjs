/// <reference path="./sdk/api.d.ts" />
import {operationFromInput,prepare,confirmOriginal,submitted,sendFailure} from './lib/selection.mjs';
/** @param {ScriptContext} ctx */
export default function main(ctx) {
  const operation=operationFromInput(ctx.input);
  const plan=prepare(ctx.desktop.readSelectionText({maxUtf16Units:8192,overflow:'reject'}),operation);
  if(plan.kind!=='ready') return {status:plan.kind};
  confirmOriginal(plan,ctx.desktop.readSelectionText({maxUtf16Units:8192,overflow:'reject'}));
  // 同步取消由现有宿主/NativeHost处理；不增加脚本取消API。
  try {return submitted(ctx.desktop.sendText(plan.replacement));}
  catch(error){throw sendFailure(error);}
}

export {main};
