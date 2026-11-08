/** @param {unknown} value */
function thenable(value){return Boolean(value&&typeof /** @type {{then?:unknown}} */(value).then==='function');}
/** @param {import('../../../runtime/api.js').ActionContext} ctx */
export default async function main(ctx){
 if(ctx.input&&typeof ctx.input==='object'&&'markerOnly' in ctx.input&&ctx.input.markerOnly===true)return await ctx.desktop.getWindowGeometry();
 const before=await ctx.desktop.targetContext();
 if(ctx.input&&typeof ctx.input==='object'&&'originalOnly' in ctx.input&&ctx.input.originalOnly===true){
  const pointer=await ctx.desktop.getPointerState();const geometry=await ctx.desktop.getWindowGeometry();
  const rect=await ctx.desktop.setWindowRect({left:-700,top:100,width:720,height:480},{frame:'visible'});
  const restore=await ctx.desktop.windowAction('restore');
  const initial='withInitialPointer' in ctx.input&&ctx.input.withInitialPointer===true;
  const drag=await ctx.desktop.trackWindowDrag({mode:'move',button:'left',maxDurationMs:20,maxUpdates:1,snap:null,...(initial?{initialPointer:{x:-12345,y:6789}}:{})});
  const after=await ctx.desktop.targetContext();return {status:'original_completed',before,after,pointer,geometry,rect,restore,drag};
 }
 const expected=await ctx.desktop.getPointingRoot();
 const handoff=await ctx.desktop.handoffPointingRoot(expected,{validForMs:5000,confirmTimeoutMs:50});
 if(!handoff.requestAccepted||!handoff.foregroundConfirmed||handoff.target===null)return {status:'handoff_stopped',handoff,shapes:[]};
 const target=handoff.target;
 if(ctx.input&&typeof ctx.input==='object'&&'toggleOnly' in ctx.input&&ctx.input.toggleOnly===true){
  const call=ctx.desktop.toggleMaximizeWindowFor(target);const shape=thenable(call);const toggle=await call;
  const after=await ctx.desktop.targetContext();return {status:'toggle_completed',before,after,target,shape,toggle};
 }
 const geometryCall=ctx.desktop.getWindowGeometryFor(target);
 const geometryShape=thenable(geometryCall);
 const geometry=await geometryCall;
 const rectCall=ctx.desktop.setWindowRectFor(target,{left:-700,top:100,width:720,height:480},{frame:'visible'});
 const rectShape=thenable(rectCall);
 const rect=await rectCall;
 const restoreCall=ctx.desktop.restoreWindowFor(target);
 const restoreShape=thenable(restoreCall);
 const restore=await restoreCall;
 const dragCall=ctx.desktop.trackWindowDragFor(target,{mode:'move',button:'left',maxDurationMs:20,maxUpdates:1,snap:null});
 const dragShape=thenable(dragCall);
 const drag=await dragCall;
 const after=await ctx.desktop.targetContext();
 return {status:'completed',before,after,target,shapes:[geometryShape,rectShape,restoreShape,dragShape],geometry,rect,restore,drag};
}
