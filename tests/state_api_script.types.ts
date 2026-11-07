/// <reference path="../crates/omnitrigger-actions/runtime/script-types.d.ts" />
declare const stateCtx: ScriptContext;
const old: ScriptActionStateSnapshot = stateCtx.state.read('session','choice');
const result: ScriptActionStateCas = stateCtx.state.compareAndSwap('durable','choice',old.revision,{present:true,value:0});
if (result.current.entry.present) { const value = result.current.entry.value; void value; }
// @ts-expect-error 同步Script返回快照，没有Promise.then
old.then(()=>{});
// @ts-expect-error 不提供任意scope
stateCtx.state.read('global','choice');
// @ts-expect-error 不接受package/action覆盖
stateCtx.state.compareAndSwap('session','choice',old.revision,{present:false},{actionId:'other'});
// @ts-expect-error present=true必须携带value
stateCtx.state.compareAndSwap('session','choice',old.revision,{present:true});
