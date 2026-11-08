/// <reference path="../../../runtime/script-types.d.ts" />
/** @param {unknown} value */
function thenable(value){return Boolean(value&&typeof /** @type {{then?:unknown}} */(value).then==='function');}
/** @type {ScriptMain} */
export function main(ctx){
 if(ctx.input&&typeof ctx.input==='object'&&'markerOnly' in ctx.input&&ctx.input.markerOnly===true)return ctx.desktop.getWindowGeometry();
 const before=ctx.desktop.targetContext();
 if(ctx.input&&typeof ctx.input==='object'&&'originalOnly' in ctx.input&&ctx.input.originalOnly===true){
  const pointer=ctx.desktop.getPointerState();const geometry=ctx.desktop.getWindowGeometry();
  const rect=ctx.desktop.setWindowRect({left:-700,top:100,width:720,height:480},{frame:'visible'});
  const restore=ctx.desktop.windowAction('restore');
  const drag=ctx.desktop.trackWindowDrag({mode:'move',button:'left',maxDurationMs:20,maxUpdates:1,snap:null});
  const after=ctx.desktop.targetContext();return {status:'original_completed',before,after,pointer,geometry,rect,restore,drag};
 }
 const expected=ctx.desktop.getPointingRoot();
 const handoff=ctx.desktop.handoffPointingRoot(expected,{validForMs:5000,confirmTimeoutMs:50});
 if(!handoff.requestAccepted||!handoff.foregroundConfirmed||handoff.target===null)return {status:'handoff_stopped',handoff,shapes:[]};
 const target=handoff.target;
 if(ctx.input&&typeof ctx.input==='object'&&'toggleOnly' in ctx.input&&ctx.input.toggleOnly===true){
  const call=ctx.desktop.toggleMaximizeWindowFor(target);const shape=thenable(call);const toggle=call;
  const after=ctx.desktop.targetContext();return {status:'toggle_completed',before,after,target,shape,toggle};
 }
 const geometryCall=ctx.desktop.getWindowGeometryFor(target);
 const geometryShape=thenable(geometryCall);
 const geometry=geometryCall;
 const rectCall=ctx.desktop.setWindowRectFor(target,{left:-700,top:100,width:720,height:480},{frame:'visible'});
 const rectShape=thenable(rectCall);
 const rect=rectCall;
 const restoreCall=ctx.desktop.restoreWindowFor(target);
 const restoreShape=thenable(restoreCall);
 const restore=restoreCall;
 const dragCall=ctx.desktop.trackWindowDragFor(target,{mode:'move',button:'left',maxDurationMs:20,maxUpdates:1,snap:null});
 const dragShape=thenable(dragCall);
 const drag=dragCall;
 const after=ctx.desktop.targetContext();
 return {status:'completed',before,after,target,shapes:[geometryShape,rectShape,restoreShape,dragShape],geometry,rect,restore,drag};
}
