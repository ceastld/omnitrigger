import type {ActionContext} from './sdk/api';
export type LineOperation = 'deduplicate'|'sort';
export type LineMode = 'inspect'|'send';
export type LineInput = {mode:LineMode;operation:LineOperation;text:string};
export type LineSummary = {
  operation:LineOperation;inputLines:number;outputLines:number;removedLines:number;
  inputUtf16Units:number;outputUtf16Units:number;terminalNewline:boolean;changed:boolean;
};
export type LineTransformed = {text:string;summary:LineSummary};
export type LineTask = {mode:LineMode}&LineTransformed;
export type LineReceipt = LineSummary & (
  {status:'inspected'|'empty';inputSubmitted:false}|
  {status:'submitted';inputSubmitted:true}
);
export type SubmitReceipt = {submitted:true};
export type NodeContext = Omit<ActionContext,'input'> & {readonly input:unknown};
export type SyncContext = Omit<ScriptContext,'input'> & {readonly input:unknown};
export type SharedContext = {desktop?:{sendText(text:string):SubmitReceipt|Promise<SubmitReceipt>}};
