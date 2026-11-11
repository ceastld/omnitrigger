/// <reference path="../../crates/omnitrigger-actions/runtime/script-types.d.ts" />
export {};
declare const ctx:ScriptContext<unknown>,target:ScriptCapturedTarget;
const submitted:{submitted:true}=ctx.desktop.toggleMaximizeWindowFor(target);
// @ts-expect-error Script回执同步。
const promise:Promise<{submitted:true}>=ctx.desktop.toggleMaximizeWindowFor(target);
// @ts-expect-error 最窄接口不能覆盖operation或传入额外选项。
ctx.desktop.toggleMaximizeWindowFor(target,{operation:'restore'});
// @ts-expect-error 普通DTO不是带类型品牌的输入租约。
ctx.desktop.toggleMaximizeWindowFor({capture_id:1,context:target.context});
void [submitted,promise];

const geometry:ScriptWindowGeometry=ctx.desktop.getWindowGeometry();
const pointer:ScriptPointerState=ctx.desktop.getPointerState();
const observed:ScriptPointingRootObservation=ctx.desktop.getPointingRoot();
// @ts-expect-error 矩形必须显式提供frame。
ctx.desktop.setWindowRect({left:0,top:0,width:100,height:100});
// @ts-expect-error 未知初始锚点字段不属于接口合同。
ctx.desktop.trackWindowDrag({mode:'move',button:'left',maxDurationMs:20,maxUpdates:1,initialPointer:{x:0,y:0}});
void [geometry,pointer,observed];
