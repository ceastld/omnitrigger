import type {ActionContext,ChooseOptions} from './sdk/api';
import type {ClipboardHistoryApi,ClipboardHistoryItem,ClipboardHistoryList} from './sdk/clipboard-history-api';
export type NodeContext = Omit<ActionContext,'input'> & {readonly input:unknown};
export type SyncContext = Omit<ScriptContext,'input'> & {readonly input:unknown};
export type SharedContext = NodeContext|SyncContext;
export type Mode = 'append'|'list'|'send';
export type SelectMode = 'list'|'send';
export type EffectState = 'not_started'|'unknown';
export type Method = 'history.list'|'history.get'|'history.append'|'ui.choose'|'keyboard.text';
export type ActionMethod = Method|'script.parameters';
export type FaultInfo = {code?:unknown;effectState?:unknown};
export type HistoryActionFault = Error & {code:string;method:ActionMethod;effectState:EffectState};
export type AppendResult = {operation:'append';inserted:boolean;utf16Units:number};
export type SelectResult = {operation:SelectMode;status:'empty'|'cancelled';itemCount:number;inputSubmitted:false}
  | {operation:'list';status:'selected';itemCount:number;selectedUtf16Units:number;inputSubmitted:false}
  | {operation:'send';status:'empty_text';itemCount:number;selectedUtf16Units:0;inputSubmitted:false}
  | {operation:'send';status:'submitted';itemCount:number;selectedUtf16Units:number;inputSubmitted:true};
export type ActionResult = AppendResult|SelectResult;
export type Done<Value extends ActionResult=ActionResult> = {done:true;value:Value};
export type Arguments = {'history.list':[Required<NonNullable<Parameters<ClipboardHistoryApi['list']>[0]>>];'history.get':Parameters<ClipboardHistoryApi['get']>;'history.append':Parameters<ClipboardHistoryApi['append']>;'ui.choose':[ChooseOptions];'keyboard.text':[string]};
export type Pending<Key extends Method=Method> = {done:false;method:Key;args:Arguments[Key];resume:(value:unknown)=>Step;effectful:boolean};
export type Call = {[Key in Method]:Pending<Key>}[Method];
export type Step = Done|Call;
export type HistoryName = 'list'|'get'|'append';
export type HistoryArguments = Arguments['history.list']|Arguments['history.get']|Arguments['history.append'];
export type {ClipboardHistoryItem,ClipboardHistoryList};
