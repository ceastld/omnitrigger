import type {ActionContext,ExplorerContextSnapshot} from '../crates/omnitrigger-actions/runtime/api';

declare const node:ActionContext;
declare const script:ScriptContext;

const pending=node.desktop.explorerContext();
const nodePromise:Promise<ExplorerContextSnapshot>=pending;
async function checkNode(){
  const result=await pending;
  const directory:string|null=result.currentDirectory;
  const selected:readonly string[]=result.selectedPaths;
  // @ts-expect-error currentDirectory 允许 null，必须先收窄。
  const nonNullable:string=result.currentDirectory;
  // @ts-expect-error 快照字段只读。
  result.currentDirectory=null;
  // @ts-expect-error 选区数组只读。
  result.selectedPaths.push('D:\\synthetic');
  // @ts-expect-error 选区数组索引不可写。
  result.selectedPaths[0]='D:\\synthetic';
  // @ts-expect-error 不能把只读选区交给可变数组。
  const mutable:string[]=result.selectedPaths;
  void [directory,selected,nonNullable,mutable];
}
// @ts-expect-error Node 返回 Promise，不能直接当快照。
const nodeSnapshot:ExplorerContextSnapshot=node.desktop.explorerContext();
// @ts-expect-error Node 必须 await 后读取字段。
node.desktop.explorerContext().selectedPaths;

const sync=script.desktop.explorerContext();
const scriptSnapshot:ScriptExplorerContextSnapshot=sync;
const directory:string|null=sync.currentDirectory;
const selected:readonly string[]=sync.selectedPaths;
// @ts-expect-error Script 同步返回快照，不是 Promise。
const scriptPromise:Promise<ScriptExplorerContextSnapshot>=sync;
// @ts-expect-error currentDirectory 允许 null，必须先收窄。
const nonNullable:string=sync.currentDirectory;
// @ts-expect-error 快照字段只读。
sync.currentDirectory=null;
// @ts-expect-error 选区数组只读。
sync.selectedPaths.push('D:\\synthetic');
// @ts-expect-error 选区数组索引不可写。
sync.selectedPaths[0]='D:\\synthetic';
// @ts-expect-error 不能把只读选区交给可变数组。
const mutable:string[]=sync.selectedPaths;

// 虚拟目录可为 null；没有选区时为空数组。
const nodeEmpty:ExplorerContextSnapshot={currentDirectory:null,selectedPaths:[]};
const scriptEmpty:ScriptExplorerContextSnapshot={currentDirectory:null,selectedPaths:[]};
// @ts-expect-error 零参数 API 不接受空选项。
node.desktop.explorerContext({});
// @ts-expect-error 原 Explorer 目标不允许用 HWND 重新选择。
node.desktop.explorerContext({hwnd:'synthetic'});
// @ts-expect-error 零参数 API 不接受空选项。
script.desktop.explorerContext({});
// @ts-expect-error 原 Explorer 目标不允许用 HWND 重新选择。
script.desktop.explorerContext({hwnd:'synthetic'});

// 新能力不能覆盖两份 desktop 声明中的既有方法。
type DesktopMethod='explorerContext'|'readSelectionText'|'captureForegroundTarget'|'sendShortcutTo'|'sendTextTo'
  |'adjustVolume'|'stepBrightness'|'target'|'targetContext'|'sendShortcut'|'sendKeySequence'|'sendText'
  |'copySelectionText'|'copySelectionData'|'windowAction';
const nodeDesktop:Pick<ActionContext['desktop'],DesktopMethod>=node.desktop;
const scriptDesktop:Pick<ScriptContext['desktop'],DesktopMethod>=script.desktop;
void [nodePromise,checkNode,nodeSnapshot,scriptSnapshot,directory,selected,scriptPromise,nonNullable,mutable,
  nodeEmpty,scriptEmpty,nodeDesktop,scriptDesktop];

// 仅显式true重载，零参数原返回类型保持。
const stageNode:Promise<ExplorerContextSnapshot>=node.desktop.explorerContext({selectionFailureStage:true});
const stageScript:ScriptExplorerContextSnapshot=script.desktop.explorerContext({selectionFailureStage:true});
for (const context of [node,script]) {
  // @ts-expect-error 不是可选参数，显式undefined仍拒绝。
  context.desktop.explorerContext(undefined);
  // @ts-expect-error false不能请求阶段证明。
  context.desktop.explorerContext({selectionFailureStage:false});
  // @ts-expect-error 不接受额外选项。
  context.desktop.explorerContext({selectionFailureStage:true,other:true});
}
void [stageNode,stageScript];
