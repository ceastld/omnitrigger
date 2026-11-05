import {analyze,format} from './json.mjs';
/** @param {unknown} value @returns {value is Record<string,unknown>} */
const object=value=>value!==null&&typeof value==='object'&&!Array.isArray(value);
/** @param {unknown} value @param {readonly string[]} names @returns {value is Record<string,unknown>} */
const fields=(value,names)=>object(value)&&Reflect.ownKeys(value).length===names.length&&names.every(name=>Object.hasOwn(value,name));
/** @param {import('./json-types').JsonErrorCode} code @returns {never} */
const fail=code=>{throw Object.assign(new Error('JSON 子值选择未完成。'),{code});};

/** @param {string} value @returns {boolean} */
function keyValid(value) {
  if(value.length>1024||value.includes('\0'))return false;
  for(let i=0;i<value.length;i++) {
    const unit=value.charCodeAt(i);
    if(unit>=0xd800&&unit<=0xdbff) {
      const next=value.charCodeAt(++i);if(!(next>=0xdc00&&next<=0xdfff))return false;
    } else if(unit>=0xdc00&&unit<=0xdfff)return false;
  }
  return true;
}

/** @param {unknown} input @returns {import('./json-types').SelectionTask} */
export function prepareSelection(input) {
  if(!fields(input,['mode','text','path','format'])||!['inspect','send'].includes(/** @type {string} */ (input.mode))
    ||!['compact','pretty'].includes(/** @type {string} */ (input.format))||!Array.isArray(input.path)||input.path.length>32)fail('invalid_input');
  const path=/** @type {unknown[]} */ (input.path);
  if(Reflect.ownKeys(path).length!==path.length+1)fail('invalid_input');
  /** @type {(string|number)[]} */
  const segments=[];
  for(let i=0;i<path.length;i++) {
    if(!Object.hasOwn(path,i))fail('invalid_input');
    const value=path[i];
    if(typeof value==='string'? !keyValid(value): !Number.isSafeInteger(value)||(/** @type {number} */ (value))<0||(/** @type {number} */ (value))>9999)fail('invalid_input');
    segments.push(/** @type {string|number} */ (value));
  }
  const mode=/** @type {'inspect'|'send'} */ (input.mode),style=/** @type {import('./json-types').Format} */ (input.format);
  const parsed=analyze(/** @type {string} */ (input.text)),tokens=parsed.tokens;
  let cursor=0;
  // 输入已由既有解析器整体校验；只为最多10000个值建立索引，不创建用户键属性。
  /** @returns {import('./json-types').SelectionNode} */
  function node() {
    const start=cursor,token=tokens[cursor++];
    /** @type {import('./json-types').SelectionNode} */
    const result={start,end:cursor,count:1,kind:token==='{'?'object':token==='['?'array':token[0]==='"'?'string':token==='null'?'null':token==='true'||token==='false'?'boolean':'number'};
    if(result.kind==='object') {
      result.children=new Map();
      while(tokens[cursor]!=='}') {
        const name=/** @type {string} */ (JSON.parse(tokens[cursor++]));cursor++; // 已验证的冒号
        const child=node();result.children.set(name,child);result.count+=child.count;
        if(tokens[cursor]===',')cursor++;
      }
      cursor++;
    } else if(result.kind==='array') {
      result.children=[];
      while(tokens[cursor]!==']') {
        const child=node();result.children.push(child);result.count+=child.count;
        if(tokens[cursor]===',')cursor++;
      }
      cursor++;
    }
    result.end=cursor;return result;
  }
  let selected=node();
  for(const segment of segments) {
    if(selected.kind==='object') {
      if(typeof segment!=='string')fail('json_path_type');
      if(!(/** @type {Map<string,import('./json-types').SelectionNode>} */ (selected.children)).has(segment))fail('json_path_missing');
      selected=/** @type {import('./json-types').SelectionNode} */ ((/** @type {Map<string,import('./json-types').SelectionNode>} */ (selected.children)).get(segment));
    } else if(selected.kind==='array') {
      if(typeof segment!=='number')fail('json_path_type');
      if(segment>=(/** @type {import('./json-types').SelectionNode[]} */ (selected.children)).length)fail('json_path_missing');
      selected=(/** @type {import('./json-types').SelectionNode[]} */ (selected.children))[segment];
    } else fail('json_path_type');
  }
  const text=format(tokens.slice(selected.start,selected.end),style);
  return {mode,text,summary:{selectedKind:selected.kind,selectedNodeCount:selected.count,pathLength:segments.length,inputUtf16Units:parsed.summary.inputUtf16Units,outputUtf16Units:text.length}};
}

/** @param {import('./json-types').SelectionTask} task @param {unknown} receipt @returns {import('./json-types').SelectionSubmitted} */
export function selectionReceipt(task,receipt) {
  if(!fields(receipt,['submitted'])||receipt.submitted!==true)fail('invalid_response');
  return {status:'submitted',...task.summary,inputSubmitted:true};
}

/** @param {unknown} error @param {boolean} attempted @returns {import('./json-types').JsonActionFault} */
export function selectionError(error,attempted) {
  /** @type {readonly import('./json-types').JsonErrorCode[]} */
  const known=['invalid_input','invalid_json','json_limit','json_duplicate_key','json_output_too_large','json_path_type','json_path_missing','invalid_response','desktop_unavailable','native_bridge_unavailable','target_changed','target_unavailable','cancelled','timeout','input_failed'];
  let code=/** @type {import('./json-types').JsonErrorCode} */ ('json_selection_failed'),effectState=/** @type {import('./json-types').EffectState} */ (attempted?'unknown':'not_started');
  try {
    if(known.includes(/** @type {import('./json-types').JsonErrorCode} */ (/** @type {import('./json-types').FaultInfo} */ (error)?.code)))code=/** @type {import('./json-types').JsonErrorCode} */ (/** @type {import('./json-types').FaultInfo} */ (error).code);
    if(attempted&&['not_started','unknown'].includes(/** @type {string} */ (/** @type {import('./json-types').FaultInfo} */ (error)?.effectState)))effectState=/** @type {import('./json-types').EffectState} */ (/** @type {import('./json-types').FaultInfo} */ (error).effectState);
  } catch (_) { /* 不外传宿主错误正文或键名。 */ }
  return Object.assign(new Error('JSON 子值动作未完成，请查看错误代码。'),{code,effectState,method:/** @type {import('./json-types').JsonMethod} */ (attempted?'keyboard.text':'script.parameters')});
}
