/// <reference path="../crates/omnitrigger-actions/runtime/script-types.d.ts" />
declare const ctx:ScriptContext;
const receipt:ScriptStopOtherInstancesResult=ctx.actions.stopOtherInstances({timeoutMs:1000});
if(receipt.stoppedCount===0){const effect:'not_started'=receipt.effectState;void effect;}
else {const effect:'unknown'=receipt.effectState;const count:1|2|3=receipt.stoppedCount;void effect;void count;}
// @ts-expect-error 回执只读
receipt.stoppedCount=0;
// @ts-expect-error 不接受调用身份
ctx.actions.stopOtherInstances({package_id:'x'});
// @ts-expect-error 预算必须为数值
ctx.actions.stopOtherInstances({timeoutMs:'500'});
// @ts-expect-error Script同步返回回执
const pending:Promise<ScriptStopOtherInstancesResult>=ctx.actions.stopOtherInstances();void pending;
declare const fault:ScriptStopOtherInstancesFault;
const method:'actions.stop_other_instances'=fault.method;void method;
