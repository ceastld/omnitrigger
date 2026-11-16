import {createSession,transition,view} from './session-core.mjs';

// ActionService输入上限64KiB；为状态转移封装留余量，不改变宿主预算。
const INPUT_BYTES=60*1024,OUTPUT_BYTES=80*1024;
const errors=new Set(['invalid_input','invalid_json','json_limit','json_duplicate_key','json_output_too_large',
  'invalid_path','path_type_mismatch','path_missing','invalid_state','invalid_event','selection_required','selection_not_visible',
  'container_required','edit_required','edit_pending','stale_revision','session_closed','session_limit','output_pending',
  'output_ack_mismatch','invalid_output_ack','unsupported_operation','unsupported_output_format','string_required',
  'cannot_delete_root','output_not_transportable','session_input_too_large','session_output_too_large','session_state_too_large','non_serializable_input','invalid_page']);
/** @param {string} code @returns {never} */
function fail(code) { throw Object.assign(new Error('JSON 会话调用未完成，请查看错误代码。'),{code}); }
/** @param {unknown} x @returns {x is Record<string,unknown>} */
const object=x=>x!==null&&typeof x==='object'&&!Array.isArray(x);
/** @param {unknown} x @param {string[]} names @returns {boolean} */
const exact=(x,names)=>object(x)&&Reflect.ownKeys(x).length===names.length&&names.every(k=>Object.hasOwn(x,k));
/** JSON RPC参数守卫；不解析文档，只拒绝非数据值、循环、getter和超出元数据界限的对象。 @param {unknown} input */
function serializable(input) {
  const ancestors=new Set();let nodes=0;
  /** @param {unknown} value @param {number} depth */
  function walk(value,depth) {
    if(++nodes>32768||depth>64)fail('non_serializable_input');
    if(value===null||typeof value==='string'||typeof value==='boolean')return;
    if(typeof value==='number') { if(!Number.isFinite(value))fail('non_serializable_input');return; }
    if(typeof value!=='object')fail('non_serializable_input');
    if(ancestors.has(value))fail('non_serializable_input');
    if(!Array.isArray(value)&&![Object.prototype,null].includes(Object.getPrototypeOf(value)))fail('non_serializable_input');
    const names=Reflect.ownKeys(value),descriptors=Object.getOwnPropertyDescriptors(value);
    if(Array.isArray(value)&&names.length!==value.length+1)fail('non_serializable_input');
    if(Array.isArray(value))for(let i=0;i<value.length;i++)if(!Object.hasOwn(value,i))fail('non_serializable_input');
    ancestors.add(value);
    for(const key of names) {
      if(typeof key!=='string')fail('non_serializable_input');
      if(Array.isArray(value)&&key==='length')continue;
      const d=descriptors[key];if(!d||!Object.hasOwn(d,'value'))fail('non_serializable_input');
      walk(d.value,depth+1);
    }
    ancestors.delete(value);
  }
  walk(input,0);
}
/** @param {string} value */
function utf8Bytes(value) {
  let bytes=0;
  for(let i=0;i<value.length;i++) {
    const c=value.charCodeAt(i);
    if(c<0x80)bytes++;else if(c<0x800)bytes+=2;
    else if(c>=0xd800&&c<=0xdbff&&value.charCodeAt(i+1)>=0xdc00&&value.charCodeAt(i+1)<=0xdfff){bytes+=4;i++;}
    else bytes+=3;
  }
  return bytes;
}
/** @param {unknown} input @returns {import('./session-types').SessionPage} */
function page(input) {
  if(input===undefined)return {offset:0,limit:32};
  if(!exact(input,['offset','limit']))fail('invalid_page');
  const p=/** @type {import('./session-types').SessionPage} */(input);
  if(!Number.isInteger(p.offset)||p.offset<0||p.offset>10000||!Number.isInteger(p.limit)||p.limit<1||p.limit>64)fail('invalid_page');
  return p;
}
/** @param {unknown} error @returns {import('./session-types').SessionFault} */
function sanitized(error) {
  let code='json_session_failed';
  try {const candidate=/** @type {{code?:unknown}} */(error)?.code;if(typeof candidate==='string'&&errors.has(candidate))code=candidate;}catch{}
  return Object.assign(new Error('JSON 会话调用未完成，请查看错误代码。'),{code,method:/** @type {const} */('script.parameters'),effectState:/** @type {const} */('not_started')});
}
/** 一次调用仅一个显式状态转移；不调用ctx中的任何副作用能力。 @param {import('./session-types').SessionContext} ctx @returns {import('./session-types').SessionResult} */
export function main(ctx) {
  try {
    const input=ctx.input;serializable(input);
    const encoded=JSON.stringify(input);if(typeof encoded!=='string')fail('non_serializable_input');
    if(utf8Bytes(encoded)>INPUT_BYTES)fail('session_input_too_large');
    if(!object(input)||!['create','step'].includes(/** @type {string} */(input.mode)))fail('invalid_input');
    const names=input.mode==='create'?['mode','initial']:['mode','state','event'];
    if(Object.hasOwn(input,'page'))names.push('page');if(!exact(input,names))fail('invalid_input');
    const request=/** @type {import('./session-types').SessionInput} */(/** @type {unknown} */(input)),requestedPage=page(request.page);
    /** @type {import('./session-types').SessionState} */let state;
    /** @type {readonly import('./session-types').SessionTextRequest[]} */let requests=[];
    if(request.mode==='create')state=createSession(request.initial);
    else {const result=transition(request.state,request.event);state=result.state;requests=result.effects;}
    const current=view(state);if(requestedPage.offset>current.rows.length)fail('invalid_page');
    const rows=current.rows.slice(requestedPage.offset,requestedPage.offset+requestedPage.limit)
      .map(row=>({...row,preview:Array.from(row.preview).slice(0,160).join('')}));
    /** @type {import('./session-types').SessionResult} */
    const result={schema_version:1,status:request.mode==='create'?'created':'stepped',state,
      view:{...current,rows,totalRows:current.rows.length,...requestedPage,previewLimit:160,previewsAreSummaries:true},
      requests,effectsExecuted:false};
    const encodedResult=JSON.stringify(result);if(typeof encodedResult!=='string')fail('json_session_failed');
    if(utf8Bytes(encodedResult)>OUTPUT_BYTES)fail('session_output_too_large');
    // 返回的state必须仍能通过同一动作提交取消/确认等控制事件。
    const continuation=JSON.stringify({mode:'step',state,event:{type:'cancel'}});
    if(utf8Bytes(continuation)>INPUT_BYTES)fail('session_state_too_large');
    if(state.pending!==null) {
      const acknowledgement=JSON.stringify({mode:'step',state,event:{type:'ack_output',requestId:state.pending.id,status:'confirmed'}});
      if(utf8Bytes(acknowledgement)>INPUT_BYTES)fail('session_state_too_large');
    }
    return result;
  } catch(error) { throw sanitized(error); }
}
