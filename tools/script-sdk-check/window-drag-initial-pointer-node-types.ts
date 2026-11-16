import type {ActionContext,CapturedTarget,WindowDragOptions} from '../../crates/omnitrigger-actions/runtime/api';
declare const ctx:ActionContext;
declare const target:CapturedTarget;
const base={mode:'move',button:'left',maxDurationMs:20,maxUpdates:1} as const;
const point={x:-700,y:200} as const;
const options:WindowDragOptions={...base,initialPointer:point};
const ordinary:Promise<unknown>=ctx.desktop.trackWindowDrag(options);
const explicit:Promise<unknown>=ctx.desktop.trackWindowDragFor(target,options);
ctx.desktop.trackWindowDrag(base);
// @ts-expect-error 起点对象只允许x/y，不接受按钮证明
ctx.desktop.trackWindowDrag({...base,initialPointer:{x:0,y:0,button:'down'}});
// @ts-expect-error null不代表默认起点
ctx.desktop.trackWindowDrag({...base,initialPointer:null});
// @ts-expect-error 两轴缺一不可
ctx.desktop.trackWindowDragFor(target,{...base,initialPointer:{x:0}});
// @ts-expect-error 不接受字符串坐标
ctx.desktop.trackWindowDragFor(target,{...base,initialPointer:{x:'0',y:0}});
