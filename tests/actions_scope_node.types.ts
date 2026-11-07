import type {ActionContext,StopOtherInstancesResult,StopOtherInstancesFault} from '../crates/omnitrigger-actions/runtime/api';
declare const ctx:ActionContext;
const pending:Promise<StopOtherInstancesResult>=ctx.actions.stopOtherInstances({timeoutMs:1000});
async function inspect(){
  const receipt=await pending;
  if(receipt.stoppedCount===0){const effect:'not_started'=receipt.effectState;void effect;}
  else {const effect:'unknown'=receipt.effectState;const count:1|2|3=receipt.stoppedCount;void effect;void count;}
  // @ts-expect-error 回执只读
  receipt.stoppedCount=0;
}
void inspect;
declare const fault:StopOtherInstancesFault;
const method:'actions.stop_other_instances'=fault.method;void method;
// @ts-expect-error 不接受调用身份
ctx.actions.stopOtherInstances({invocation_id:'1'});
// @ts-expect-error 预算必须为数值
ctx.actions.stopOtherInstances({timeoutMs:'500'});
// @ts-expect-error Node必须等待Promise
const sync:StopOtherInstancesResult=ctx.actions.stopOtherInstances();void sync;
