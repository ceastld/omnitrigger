import {configuration,prepareCall,packageResult,launched,cancelled} from './lib.mjs';
/** @param {import('./types/api.d.ts').ActionContext} ctx */
export default async function main(ctx) {
  const {mode,directory}=configuration(ctx.input);
  cancelled(ctx.signal);
  if(mode!=='verify') {
    const resident=await ctx.desktop.findRaiShotResident();
    cancelled(ctx.signal);
    if(resident) {
      if(mode==='tray') return {status:'resident',mode,screenshotCompleted:false};
      const queued=await ctx.desktop.requestRaiShotRegion(resident);
      return {status:'queued',mode,...queued};
    }
  }
  const call=prepareCall(ctx.packageRoot,mode,directory,ctx.input.powerShellProgram);
  const pkg=packageResult(await ctx.process.capture(call.program,call.args,call.options),mode);
  cancelled(ctx.signal);
  if(mode==='verify') return {status:'verified',...pkg};
  return launched(await ctx.process.start(pkg.exe_path,[`--${mode}`],{cwd:pkg.directory,windowsHide:false}),mode);
}
