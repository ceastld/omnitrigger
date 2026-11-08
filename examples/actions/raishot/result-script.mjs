/// <reference path="./sdk/api.d.ts" />
/** @param {ScriptContext} ctx */
export default function main(ctx) {
  const input=ctx.input;
  const keys=Object.keys(input);
  if(input.operation==='release'&&keys.length===2&&keys.includes('requestId'))
    return ctx.desktop.releaseRaiShotResult(input.requestId);
  if(input.operation==='capture'&&keys.length===2&&keys.includes('timeoutMs'))
    return ctx.desktop.captureRaiShotResult({timeoutMs:input.timeoutMs});
  throw Object.assign(new Error('需要 capture/timeoutMs 或 release/requestId。'),
    {code:'invalid_input',effectState:'not_started',method:'raishot.capture_result'});
}
export {main};
