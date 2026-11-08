import type {ActionContext,PointingRootObservation,PointingRootHandoffOptions,WindowGeometry,WindowRect} from './sdk/api.js';
export type Rect=WindowRect;
export type Maybe<T>=T|Promise<T>;
export type Identity=Readonly<{window_handle:string;process_id:number;thread_id:number;process_created_file_time:string}>;
export type Original=Identity&Readonly<{expires_at_unix_ms:number}>;
export type SizeInput=Readonly<{trigger:'AdvancedMouseAction'|'PowerKeys';step?:number;targetPolicy?:'original-only'|'handoff-pointing-root';handoff?:PointingRootHandoffOptions}>;
export type ParsedInput=Readonly<{trigger:SizeInput['trigger'];step:number;targetPolicy:'original-only'|'handoff-pointing-root';handoff:PointingRootHandoffOptions|null}>;
export interface SizeContext<T extends {context:Original}> {
 input:unknown;signal?:{readonly aborted:boolean};
 desktop:{
  targetContext():Maybe<Original>;getPointingRoot():Maybe<PointingRootObservation>;
  handoffPointingRoot(expected:PointingRootObservation,options:PointingRootHandoffOptions):Maybe<Readonly<{requested:true;requestAccepted:boolean;foregroundConfirmed:boolean;target:T|null;reason:null|'os_rejected'|'confirmation_timeout';effectState:'unknown'}>>;
  getWindowGeometry():Maybe<WindowGeometry>;getWindowGeometryFor(target:T):Maybe<WindowGeometry>;
  windowAction(operation:'restore'):Maybe<{submitted:true}>;restoreWindowFor(target:T):Maybe<{submitted:true}>;
  setWindowRect(rect:WindowRect,options:{frame:'visible'}):Maybe<{submitted:true;effectState:'unknown'}>;
  setWindowRectFor(target:T,rect:WindowRect,options:{frame:'visible'}):Maybe<{submitted:true;effectState:'unknown'}>;
 };
}
export type SizeResult=Readonly<{
 status:'pointing_target_differs'|'handoff_not_confirmed'|'source_class_blocked'|'restore_pending'|'size_submitted';
 operation:'increase'|'decrease';targetRoute:'original'|'captured';effectState:'not_started'|'unknown';submitted:boolean;windowChangeConfirmed:false;
 originalTarget:Identity;selectedTarget:Identity;className:string|null;plannedRect:WindowRect|null;primaryScreen:Readonly<{width:number;height:number}>|null;
 noOpReason:'exact_platform_class'|'source_pattern'|null;restoreRequested:boolean;handoff:Readonly<{requestAccepted:boolean;foregroundConfirmed:boolean;reason:null|'os_rejected'|'confirmation_timeout'}>|null;
}>;
export type NodeContext=ActionContext;
