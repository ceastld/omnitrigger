// 原创纯会话层。只复用本项目MIT JSON词法校验/排版模块，不含来源动作代码。
import {analyze,format} from './json.mjs';

function fail(code) { throw Object.assign(new Error('JSON 会话未完成，请查看错误代码。'),{code,effectState:'not_started'}); }
const record=x=>x!==null&&typeof x==='object'&&!Array.isArray(x);
function fields(x,names) { return record(x)&&Reflect.ownKeys(x).length===names.length&&names.every(k=>Object.hasOwn(x,k)); }
const equalPath=(a,b)=>a.length===b.length&&a.every((x,i)=>x===b[i]);
function outputText(value) {
  if(typeof value!=='string'||value.length>65536||value.includes('\0'))fail('output_not_transportable');
  for(let i=0;i<value.length;i++) {
    const unit=value.charCodeAt(i);
    if(unit>=0xd800&&unit<=0xdbff) {
      const next=value.charCodeAt(++i);if(!(next>=0xdc00&&next<=0xdfff))fail('output_not_transportable');
    } else if(unit>=0xdc00&&unit<=0xdfff)fail('output_not_transportable');
  }
  return value;
}
function path(value) {
  if(!Array.isArray(value)||value.length>32||Reflect.ownKeys(value).length!==value.length+1)fail('invalid_path');
  for(let i=0;i<value.length;i++) {
    if(!Object.hasOwn(value,i))fail('invalid_path');
    const item=value[i];
    if(typeof item==='string'?item.length>1024:!Number.isSafeInteger(item)||item<0||item>9999)fail('invalid_path');
  }
  return value.slice();
}
export function pointer(segments) {
  return path(segments).map(x=>'/'+String(x).replaceAll('~','~0').replaceAll('/','~1')).join('');
}

// 词法/重复键/数值范围资源限制由既有analyze负责；此索引只定位已校验token区间。
function index(text) {
  const {tokens}=analyze(text);let i=0;
  function node() {
    const start=i,t=tokens[i++];
    const kind=t==='{'?'object':t==='['?'array':t.startsWith('"')?'string':t==='null'?'null':t==='true'||t==='false'?'boolean':'number';
    const result={start,end:i,kind,children:[]};
    if(kind==='object'||kind==='array') {
      const close=kind==='object'?'}':']';
      while(tokens[i]!==close) {
        const entryStart=i;
        const segment=kind==='object'?JSON.parse(tokens[i++]):result.children.length;
        if(kind==='object')i++; // 已校验的冒号
        const value=node();result.children.push({segment,start:entryStart,end:value.end,value});
        if(tokens[i]===',')i++;
      }
      i++;
    }
    result.end=i;return result;
  }
  const root=node();
  function at(segments) {
    let value=root;
    for(const segment of path(segments)) {
      if(!['object','array'].includes(value.kind)||value.kind==='object'&&typeof segment!=='string'||value.kind==='array'&&typeof segment!=='number')fail('path_type_mismatch');
      const child=value.children.find(c=>c.segment===segment);
      if(!child)fail('path_missing');
      value=child.value;
    }
    return value;
  }
  return {tokens,root,at};
}
function snapshot(s) {
  const out={...s,browsePath:s.browsePath.slice(),selectionPath:s.selectionPath?.slice()??null,
    edit:s.edit?{...s.edit,path:s.edit.path.slice()}:null,pending:s.pending?{...s.pending}:null,
    lastOutput:s.lastOutput?{...s.lastOutput}:null};
  Object.freeze(out.browsePath);if(out.selectionPath)Object.freeze(out.selectionPath);
  if(out.edit){Object.freeze(out.edit.path);Object.freeze(out.edit);}
  if(out.pending)Object.freeze(out.pending);if(out.lastOutput)Object.freeze(out.lastOutput);
  return Object.freeze(out);
}
function validate(s) {
  if(!fields(s,['schemaVersion','sessionId','revision','text','browsePath','selectionPath','mode','edit','pending','continuous','outputSequence','lastOutput','endReason'])
    ||s.schemaVersion!==1||typeof s.sessionId!=='string'||!/^[A-Za-z0-9_-]{1,64}$/.test(s.sessionId)
    ||!Number.isSafeInteger(s.revision)||s.revision<0||s.revision>100000
    ||!Number.isSafeInteger(s.outputSequence)||s.outputSequence<0||s.outputSequence>100000
    ||!['browse','edit','pending','closed'].includes(s.mode)||typeof s.continuous!=='boolean'
    ||(s.endReason!==null&&!['cancelled','output_confirmed','output_unknown'].includes(s.endReason)))fail('invalid_state');
  path(s.browsePath);if(s.selectionPath!==null)path(s.selectionPath);
  if(s.edit!==null&&(!fields(s.edit,['revision','path'])||s.edit.revision!==s.revision))fail('invalid_state');
  if(s.edit)path(s.edit.path);
  if(s.mode==='edit'&&s.edit===null||s.mode!=='edit'&&s.edit!==null)fail('invalid_state');
  if(s.pending!==null&&(!fields(s.pending,['id','revision','kind','text','closeOnSuccess'])||s.pending.revision!==s.revision
    ||typeof s.pending.text!=='string'||s.pending.text.length>65536||typeof s.pending.closeOnSuccess!=='boolean'
    ||!['path','value','document'].includes(s.pending.kind)||s.pending.id!==s.sessionId+':'+s.outputSequence))fail('invalid_state');
  if(s.mode==='pending'&&s.pending===null||s.pending!==null&&!['pending','closed'].includes(s.mode))fail('invalid_state');
  if(s.lastOutput!==null&&(!fields(s.lastOutput,['id','status'])||typeof s.lastOutput.id!=='string'
    ||!['confirmed','not_started','unknown'].includes(s.lastOutput.status)))fail('invalid_state');
  return index(s.text);
}
export function createSession(input) {
  if(!fields(input,['sessionId','text','continuous'])||typeof input.sessionId!=='string'
    ||!/^[A-Za-z0-9_-]{1,64}$/.test(input.sessionId)||typeof input.continuous!=='boolean')fail('invalid_input');
  index(input.text); // 不把坏JSON静默替换成空对象，不读保存文本或剪贴板。
  return snapshot({schemaVersion:1,sessionId:input.sessionId,revision:0,text:input.text,browsePath:[],selectionPath:null,
    mode:'browse',edit:null,pending:null,continuous:input.continuous,outputSequence:0,lastOutput:null,endReason:null});
}
export function view(s) {
  const tree=validate(s),current=tree.at(s.browsePath);
  const children=current.children.length?current.children.map(c=>({segment:c.segment,node:c.value})):[];
  const rows=children.map(c=>({path:[...s.browsePath,c.segment],label:String(c.segment),kind:c.node.kind,
    preview:Array.from(format(tree.tokens.slice(c.node.start,c.node.end),'compact')).slice(0,160).join('')}));
  if(!['object','array'].includes(current.kind))rows.push({path:s.browsePath.slice(),label:'当前值',kind:current.kind,
    preview:format(tree.tokens.slice(current.start,current.end),'compact')});
  return {revision:s.revision,mode:s.mode,browsePath:s.browsePath.slice(),pointer:pointer(s.browsePath),rows};
}
function eventFields(event,names) {
  if(!fields(event,['type',...names]))fail('invalid_event');
}
function selection(s,tree) {
  if(s.selectionPath===null)fail('selection_required');
  return tree.at(s.selectionPath);
}
export function transition(s,event) {
  const tree=validate(s);if(!record(event)||typeof event.type!=='string')fail('invalid_event');
  let next={...s};const effects=[];
  if(event.type==='cancel') {
    eventFields(event,[]);if(s.mode==='closed')return {state:s,effects};
    next.mode='closed';next.edit=null;next.endReason='cancelled';
    // 已发出的输出不能据取消推断未写入；保留pending，允许晚到回执只记账、不恢复会话。
    return {state:snapshot(next),effects};
  }
  if(event.type==='ack_output') {
    eventFields(event,['requestId','status']);
    if(!s.pending||event.requestId!==s.pending.id)fail('output_ack_mismatch');
    if(!['confirmed','not_started','unknown'].includes(event.status))fail('invalid_output_ack');
    next.lastOutput={id:event.requestId,status:event.status};next.pending=null;
    if(s.mode!=='closed') {
      if(event.status==='unknown'){next.mode='closed';next.endReason='output_unknown';}
      else if(event.status==='confirmed'&&s.pending.closeOnSuccess){next.mode='closed';next.endReason='output_confirmed';}
      else next.mode='browse';
    }
    return {state:snapshot(next),effects};
  }
  if(s.mode==='closed')fail('session_closed');
  if(s.mode==='pending')fail('output_pending');
  if(event.revision!==s.revision)fail('stale_revision');
  if(s.mode==='edit'&&!['commit_edit','cancel_edit'].includes(event.type))fail('edit_pending');
  switch(event.type) {
    case 'select': {
      eventFields(event,['revision','path']);const p=path(event.path);tree.at(p);
      if(!(p.length===s.browsePath.length+1&&equalPath(p.slice(0,-1),s.browsePath))
        &&!(equalPath(p,s.browsePath)&&!['object','array'].includes(tree.at(p).kind)))fail('selection_not_visible');
      next.selectionPath=p;break;
    }
    case 'open_selected': {
      eventFields(event,['revision']);const n=selection(s,tree);
      if(!['object','array'].includes(n.kind))fail('container_required');
      next.browsePath=s.selectionPath.slice();next.selectionPath=null;break;
    }
    case 'back':eventFields(event,['revision']);next.browsePath=s.browsePath.slice(0,-1);next.selectionPath=null;break;
    case 'home':eventFields(event,['revision']);next.browsePath=[];next.selectionPath=null;break;
    case 'set_continuous':
      eventFields(event,['revision','value']);if(typeof event.value!=='boolean')fail('invalid_event');next.continuous=event.value;break;
    case 'begin_edit':
      eventFields(event,['revision']);selection(s,tree);next.mode='edit';next.edit={revision:s.revision,path:s.selectionPath.slice()};break;
    case 'cancel_edit':
      eventFields(event,['revision']);if(!s.edit)fail('edit_required');next.mode='browse';next.edit=null;break;
    case 'commit_edit': {
      eventFields(event,['revision','replacementText']);if(!s.edit)fail('edit_required');
      const replacement=index(event.replacementText),target=tree.at(s.edit.path);
      const tokens=[...tree.tokens.slice(0,target.start),...replacement.tokens,...tree.tokens.slice(target.end)];
      const changed=format(tokens,'compact');index(changed);
      next.text=changed;next.revision++;next.mode='browse';next.edit=null;break;
    }
    case 'delete_selected': {
      eventFields(event,['revision']);selection(s,tree);if(s.selectionPath.length===0)fail('cannot_delete_root');
      const parent=tree.at(s.selectionPath.slice(0,-1)),segment=s.selectionPath[s.selectionPath.length-1];
      const i=parent.children.findIndex(c=>c.segment===segment),entry=parent.children[i];
      const start=i===parent.children.length-1&&i>0?entry.start-1:entry.start;
      const end=i<parent.children.length-1?entry.end+1:entry.end;
      next.text=format([...tree.tokens.slice(0,start),...tree.tokens.slice(end)],'compact');index(next.text);
      next.revision++;next.selectionPath=null;break;
    }
    case 'load_document':
      eventFields(event,['revision','text']);index(event.text);next.text=event.text;next.revision++;next.browsePath=[];next.selectionPath=null;break;
    case 'request_output': {
      eventFields(event,['revision','kind','format']);if(s.outputSequence===100000)fail('session_limit');
      let payload;
      if(event.kind==='path') {
        selection(s,tree);if(event.format!=='pointer')fail('unsupported_output_format');payload=pointer(s.selectionPath);
      } else if(event.kind==='value'||event.kind==='document') {
        const node=event.kind==='document'?tree.root:selection(s,tree);
        if(event.format==='raw-string') {
          if(event.kind==='document')fail('unsupported_output_format');
          if(node.kind!=='string')fail('string_required');payload=JSON.parse(tree.tokens[node.start]);
        } else payload=format(tree.tokens.slice(node.start,node.end),event.format);
      } else fail('invalid_event');
      outputText(payload);
      const pending={id:s.sessionId+':'+(s.outputSequence+1),revision:s.revision,kind:event.kind,text:payload,
        closeOnSuccess:event.kind==='document'||!s.continuous};
      next.mode='pending';next.outputSequence++;next.pending=pending;
      effects.push({type:'text_output_requested',requestId:pending.id,revision:s.revision,text:payload,
        payloadFormat:event.format});break;
    }
    default:fail('unsupported_operation');
  }
  if(next.revision>100000)fail('session_limit');
  return {state:snapshot(next),effects};
}

/** 独立显式类型转换；不采用原动作的隐式Date文化解析或坏JSON回退。 */
export function encodeEditValue(input) {
  if(!fields(input,['kind','value']))fail('invalid_input');
  if(input.kind==='string') {
    if(typeof input.value!=='string')fail('invalid_edit_value');const out=JSON.stringify(input.value);index(out);return out;
  }
  if(input.kind==='boolean') {if(typeof input.value!=='boolean')fail('invalid_edit_value');return input.value?'true':'false';}
  if(input.kind==='null') {if(input.value!==null)fail('invalid_edit_value');return 'null';}
  if(!['object','array','number','int64'].includes(input.kind)||typeof input.value!=='string')fail('unsupported_edit_kind');
  const parsed=index(input.value);
  if(input.kind==='int64') {
    if(!/^-?(0|[1-9][0-9]*)$/.test(input.value))fail('invalid_int64');
    const negative=input.value[0]==='-',digits=negative?input.value.slice(1):input.value;
    const limit=negative?'9223372036854775808':'9223372036854775807';
    if(digits.length>limit.length||digits.length===limit.length&&digits>limit)fail('invalid_int64');
  } else if(parsed.root.kind!==input.kind)fail('edit_kind_mismatch');
  return format(parsed.tokens,'compact');
}
