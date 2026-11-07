/// <reference path="./sdk/api.d.ts" />
import {configuration,prepareCall,packageResult,launched} from './lib.mjs';
/** @param {ScriptContext} ctx */
export default function main(ctx) {
  const {mode,directory}=configuration(ctx.input);
  if(mode!=='verify') {
    const resident=ctx.desktop.findRaiShotResident();
    if(resident) {
      if(mode==='tray') return {status:'resident',mode,screenshotCompleted:false};
      return {status:'queued',mode,...ctx.desktop.requestRaiShotRegion(resident)};
    }
  }
  const call=prepareCall(ctx.packageRoot,mode,directory,ctx.input.powerShellProgram);
  const pkg=packageResult(ctx.process.capture(call.program,call.args,call.options),mode);
  if(mode==='verify') return {status:'verified',...pkg};
  return launched(ctx.process.start(pkg.exe_path,[`--${mode}`]),mode);
}
export {main};
