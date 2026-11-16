import type {SessionInitial,SessionState,SessionEvent,SessionPath,CoreSessionView,SessionTextRequest,EditValue} from './session-types';
export function createSession(input:SessionInitial):SessionState;
export function transition(state:SessionState,event:SessionEvent):{state:SessionState;effects:SessionTextRequest[]};
export function view(state:SessionState):CoreSessionView;
export function pointer(path:SessionPath):string;
export function encodeEditValue(input:EditValue):string;
