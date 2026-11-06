// Node使用default async；轻量宿主使用同步named main。两入口共享同一读取逻辑与合成冒烟分支。
function failure(cause){return {kind:'error',code:cause?.code,effectState:cause?.effectState};}
function selection(value){return {kind:'selection',text:value.text,truncated:value.truncated};}
export function main(ctx){
  if(ctx.input.operation==='entry_smoke'){
    return {kind:'entry_smoke',sentinel:ctx.input.sentinel,hasSelectionRead:typeof ctx.desktop?.readSelectionText==='function'};
  }
  try {
    const value=ctx.desktop.readSelectionText(ctx.input.overflow?{overflow:ctx.input.overflow}:undefined);
    return value&&typeof value.then==='function'?value.then(selection,failure):selection(value);
  } catch(cause){return failure(cause);}
}
export default async function(ctx){return main(ctx);}
