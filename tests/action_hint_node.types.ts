import type {ActionContext} from '../crates/omnitrigger-actions/runtime/api';
declare const ctx:ActionContext;
async function contract() {
  const result=await ctx.ui.setActionHint({text:'v123'});
  const updated:true=result.updated;
  const snapshot=await ctx.state.read('session','hint');
  await ctx.state.compareAndSwap('durable','hint',snapshot.revision,{present:true,value:0});
  await ctx.actions.stopOtherInstances({timeoutMs:1});
  // @ts-expect-error Node返回Promise，不能直接读取回执。
  ctx.ui.setActionHint({text:'v123'}).updated;
  // @ts-expect-error 参数只能是text对象。
  ctx.ui.setActionHint('v123');
  // @ts-expect-error text必须为字符串。
  ctx.ui.setActionHint({text:123});
  // @ts-expect-error 不能指定其他调用身份。
  ctx.ui.setActionHint({text:'v123',bindingId:1});
  // @ts-expect-error 不接受样式参数。
  ctx.ui.setActionHint({text:'v123',color:'green'});
  return updated;
}
void contract;
