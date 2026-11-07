import type {ActionContext,ActionStateSnapshot,ActionStateCas} from '../crates/omnitrigger-actions/runtime/api.js';
declare const stateCtx: ActionContext;
async function contract() {
  const pending: Promise<ActionStateSnapshot> = stateCtx.state.read('session','choice');
  const old = await pending;
  const result: ActionStateCas = await stateCtx.state.compareAndSwap('session','choice',old.revision,{present:true,value:null});
  if (result.current.entry.present) { const value = result.current.entry.value; void value; }
  else {
    // @ts-expect-error missing没有value
    result.current.entry.value;
  }
  // @ts-expect-error 不提供任意scope
  stateCtx.state.read('global','choice');
  // @ts-expect-error 不接受actionId覆盖
  stateCtx.state.read('session','choice',{actionId:'other'});
  // @ts-expect-error present=false不能携带value
  stateCtx.state.compareAndSwap('durable','choice',old.revision,{present:false,value:0});
}
void contract;
