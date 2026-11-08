/** 单次普通Script调用的数据合同；文档必须为text，不接受已可能舍入的JS数值对象。 */
export type SessionPath = readonly (string|number)[];
export type SessionKind = 'object'|'array'|'string'|'number'|'boolean'|'null';
export type SessionMode = 'browse'|'edit'|'pending'|'closed';
export type OutputStatus = 'confirmed'|'not_started'|'unknown';
export type SessionInitial = Readonly<{sessionId:string;text:string;continuous:boolean}>;
export type PendingOutput = Readonly<{id:string;revision:number;kind:'path'|'value'|'document';text:string;closeOnSuccess:boolean}>;
export type SessionState = Readonly<{
  schemaVersion:1;sessionId:string;revision:number;text:string;browsePath:SessionPath;selectionPath:SessionPath|null;
  mode:SessionMode;edit:Readonly<{revision:number;path:SessionPath}>|null;pending:PendingOutput|null;
  continuous:boolean;outputSequence:number;lastOutput:Readonly<{id:string;status:OutputStatus}>|null;
  endReason:'cancelled'|'output_confirmed'|'output_unknown'|null;
}>;
export type RevisionEvent = Readonly<{revision:number}>;
export type SessionEvent =
  | (RevisionEvent & Readonly<{type:'select';path:SessionPath}>)
  | (RevisionEvent & Readonly<{type:'open_selected'|'back'|'home'|'begin_edit'|'cancel_edit'|'delete_selected'}>)
  | (RevisionEvent & Readonly<{type:'set_continuous';value:boolean}>)
  | (RevisionEvent & Readonly<{type:'commit_edit';replacementText:string}>)
  | (RevisionEvent & Readonly<{type:'load_document';text:string}>)
  | (RevisionEvent & Readonly<{type:'request_output';kind:'path';format:'pointer'}>)
  | (RevisionEvent & Readonly<{type:'request_output';kind:'value';format:'compact'|'pretty'|'raw-string'}>)
  | (RevisionEvent & Readonly<{type:'request_output';kind:'document';format:'compact'|'pretty'}>)
  | Readonly<{type:'ack_output';requestId:string;status:OutputStatus}>
  | Readonly<{type:'cancel'}>;
export type SessionTextRequest = Readonly<{type:'text_output_requested';requestId:string;revision:number;text:string;payloadFormat:'pointer'|'compact'|'pretty'|'raw-string'}>;
export type SessionRow = Readonly<{path:SessionPath;label:string;kind:SessionKind;preview:string}>;
export type CoreSessionView = Readonly<{revision:number;mode:SessionMode;browsePath:SessionPath;pointer:string;rows:readonly SessionRow[]}>;
export type SessionPage = Readonly<{offset:number;limit:number}>;
export type SessionView = Omit<CoreSessionView,'rows'> & Readonly<{
  rows:readonly SessionRow[];totalRows:number;offset:number;limit:number;previewLimit:160;previewsAreSummaries:true;
}>;
export type SessionInput =
  | Readonly<{mode:'create';initial:SessionInitial;page?:SessionPage}>
  | Readonly<{mode:'step';state:SessionState;event:SessionEvent;page?:SessionPage}>;
export type SessionResult = Readonly<{
  schema_version:1;status:'created'|'stepped';state:SessionState;view:SessionView;requests:readonly SessionTextRequest[];
  effectsExecuted:false;
}>;
export type SessionContext = Readonly<{input:unknown}>;
export type EditValue = Readonly<{kind:'string';value:string}>|Readonly<{kind:'boolean';value:boolean}>
  |Readonly<{kind:'null';value:null}>|Readonly<{kind:'object'|'array'|'number'|'int64';value:string}>;
export interface SessionFault extends Error {code:string;method:'script.parameters';effectState:'not_started'}
