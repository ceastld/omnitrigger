/** @param {unknown} value */
function options(value){
 if(!value||typeof value!=='object'||Array.isArray(value))throw Object.assign(Error('需要明确 mode、handoff、restore。'),{code:'invalid_input',effectState:'not_started'});
 const fields=Object.getOwnPropertyDescriptors(value),keys=Reflect.ownKeys(fields);
 if(keys.length!==3||!['mode','handoff','restore'].every(k=>Object.hasOwn(fields,k)&&Object.hasOwn(fields[k],'value')))throw Object.assign(Error('调整参数无效。'),{code:'invalid_input',effectState:'not_started'});
 const result={mode:fields.mode.value,handoff:fields.handoff.value,restore:fields.restore.value};
 if(!['move','resize'].includes(result.mode)||typeof result.handoff!=='boolean'||typeof result.restore!=='boolean')throw Object.assign(Error('调整参数无效。'),{code:'invalid_input',effectState:'not_started'});
 return Object.freeze(/** @type {{mode:'move'|'resize';handoff:boolean;restore:boolean}} */(result));
}
/** 单次有界调整；不注入/消费按钮，不把停止回执称为完整拖拽。
 * @param {import('../../../crates/omnitrigger-actions/runtime/api.js').ActionContext} ctx */
export default async function main(ctx){
 const input=options(ctx.input);
 /** @type {'not_started'|'unknown'} */let effect='not_started';let writing=false;
 const check=()=>{if(ctx.signal.aborted)throw Object.assign(Error('调整已取消。'),{code:'cancelled',effectState:effect});};
 try{
  check();const original=await ctx.desktop.targetContext();check();
  const pointing=await ctx.desktop.getPointingRoot();check();
  const same=['window_handle','process_id','thread_id','process_created_file_time'].every(k=>original[/** @type {keyof typeof original} */(k)]===pointing.rootWindow[/** @type {keyof typeof pointing.rootWindow} */(k)]);
  /** @type {import('../../../crates/omnitrigger-actions/runtime/api.js').CapturedTarget|null} */let target=null;
  if(!same){
   if(!input.handoff)return {status:'different_pointing_root',effectState:effect,dragCompleted:false};
   writing=true;
   const receipt=await ctx.desktop.handoffPointingRoot(pointing,{validForMs:5000,confirmTimeoutMs:50});effect='unknown';writing=false;check();
   if(!receipt.requestAccepted||!receipt.foregroundConfirmed||receipt.target===null)return {status:'handoff_stopped',effectState:effect,dragCompleted:false};
   target=receipt.target;
  }
  const read=()=>target===null?ctx.desktop.getWindowGeometry():ctx.desktop.getWindowGeometryFor(target);
  let geometry=await read();check();
  if(geometry.maximized){
   if(!input.restore)return {status:'restore_required',effectState:effect,dragCompleted:false};
   writing=true;
   if(target===null)await ctx.desktop.windowAction('restore');else await ctx.desktop.restoreWindowFor(target);
   effect='unknown';writing=false;check();geometry=await read();check();
   if(geometry.maximized)return {status:'restore_pending',effectState:effect,dragCompleted:false};
  }
  const adjustment={mode:input.mode,button:/** @type {const} */('left'),maxDurationMs:1000,maxUpdates:32,snap:null};
  writing=true;
  const result=target===null?await ctx.desktop.trackWindowDrag(adjustment):await ctx.desktop.trackWindowDragFor(target,adjustment);
  if(result.effectState==='unknown')effect='unknown';writing=false;check();
  return {status:'bounded_adjustment_stopped',route:target===null?'original':'captured',effectState:effect,dragCompleted:false,result};
 }catch(cause){
  const error=/** @type {{code?:string;effectState?:string}} */(cause);
  throw Object.assign(Error('窗口调整未完成；不会自动重试或回滚。'),{code:error?.code??'window_adjust_failed',method:'script.window_adjust',effectState:effect==='unknown'||error?.effectState==='unknown'||writing&&error?.effectState!=='not_started'?'unknown':'not_started'});
 }
}
