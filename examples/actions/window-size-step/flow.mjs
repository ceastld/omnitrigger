import {decodeOriginal,decodePointing,decodeGeometry} from './read.mjs';
import {planSourceSizeStep} from './source-plan.mjs';
/** @typedef {Readonly<{resume:()=>SizeWork}>} SizeContinuation
 * @typedef {import('./types.js').SizeResult|SizeContinuation|Promise<import('./types.js').SizeResult|SizeContinuation>} SizeWork */
/** @param {string} code @param {'not_started'|'unknown'} [effectState] @returns {never} */
function fail(code,effectState='not_started'){throw Object.assign(Error('窗口大小调整条件不满足；不会自动重试。'),{code,effectState,method:'script.window_size_step'});}
/** @param {unknown} value @param {readonly string[]} names @param {readonly string[]} [optional] @returns {Record<string,any>} */
function row(value,names,optional=[]){
 if(!value||typeof value!=='object'||Array.isArray(value)||![Object.prototype,null].includes(Object.getPrototypeOf(value)))fail('invalid_input');
 const d=Object.getOwnPropertyDescriptors(value),keys=Reflect.ownKeys(d);
 if(names.some(k=>!Object.hasOwn(d,k))||keys.some(k=>typeof k!=='string'||![...names,...optional].includes(k)||!Object.hasOwn(d[k],'value')))fail('invalid_input');
 return Object.fromEntries(keys.map(k=>[k,/** @type {PropertyDescriptorMap} */(d)[/** @type {string} */(k)].value]));
}
/** @param {unknown} value @returns {import('./types.js').ParsedInput} */
export function parseInput(value){
 const r=row(value,['trigger'],['step','targetPolicy','handoff']),step=Object.hasOwn(r,'step')?r.step:50;
 const targetPolicy=Object.hasOwn(r,'targetPolicy')?r.targetPolicy:'original-only';
 if(!['AdvancedMouseAction','PowerKeys'].includes(r.trigger)||!Number.isInteger(step)||step<10||step>200||!['original-only','handoff-pointing-root'].includes(targetPolicy))fail('invalid_input');
 let handoff=null;
 if(targetPolicy==='handoff-pointing-root'){
  const h=row(r.handoff,['validForMs','confirmTimeoutMs']);
  if(!Number.isInteger(h.validForMs)||h.validForMs<2||h.validForMs>5000||!Number.isInteger(h.confirmTimeoutMs)||h.confirmTimeoutMs<1||h.confirmTimeoutMs>1000||h.confirmTimeoutMs>=h.validForMs)fail('invalid_input');
  handoff=Object.freeze({validForMs:h.validForMs,confirmTimeoutMs:h.confirmTimeoutMs});
 }else if(Object.hasOwn(r,'handoff'))fail('invalid_input');
 return Object.freeze({trigger:r.trigger,step,targetPolicy,handoff});
}
/** 业务层固定来源模式；按差分证据保留Kelvin文化歧义，其余Unicode不作整体拒绝。
 * @param {string} className */
export function sourceClassBlocked(className){
 if(typeof className!=='string'||!className||className.length>255||className.includes('\0'))fail('invalid_response');
 // 固定ASCII模式显式折叠ASCII字母，避免引擎的/i把long-s或Kelvin加入匹配。
 const folded=className.replace(/[A-Z]/g,c=>String.fromCharCode(c.charCodeAt(0)+32));
 const pattern=/progman|workerw|shell_traywnd/;
 const direct=pattern.test(folded);
 // 固定模式的BMP字面字符差分：.NET实测均不折叠long-s；Kelvin随Framework invariant文化变化。
 // 影子匹配不改原始metadata；只有Kelvin折叠会新增完整黑名单匹配时才保留文化缺口。
 if(!direct&&pattern.test(folded.replace(/\u212a/g,'k')))fail('source_regex_culture_unavailable');
 return direct;
}
/** @param {import('./types.js').Identity} a @param {import('./types.js').Identity} b */
function same(a,b){return a.window_handle===b.window_handle&&a.process_id===b.process_id&&a.thread_id===b.thread_id&&a.process_created_file_time===b.process_created_file_time;}

/** 单次来源大小步进；现有RPC和不透明Lease原样传递，绝不重捕获、重试或补偿。
 * @template {{context:import('./types.js').Original}} T
 * @param {import('./types.js').SizeContext<T>} ctx @param {boolean} asynchronous @param {'increase'|'decrease'} operation
 * @returns {import('./types.js').Maybe<import('./types.js').SizeResult>} */
export function runSizeStep(ctx,asynchronous,operation){
 if(!['increase','decrease'].includes(operation))fail('invalid_input');
 const input=parseInput(ctx.input),d=ctx.desktop;
 /** @type {'not_started'|'unknown'} */let effect='not_started';
 /** @type {'original'|'captured'} */let route='original';
 /** @type {import('./types.js').Identity} */let original;
 /** @type {import('./types.js').Identity} */let selected;
 /** @type {string|null} */let className=null;
 /** @type {import('./sdk/api.js').WindowRect|null} */let planned=null;
 /** @type {Readonly<{width:number;height:number}>|null} */let primary=null;
 /** @type {import('./types.js').SizeResult['handoff']} */let handoff=null;
 let restored=false;
 /** @type {'exact_platform_class'|'source_pattern'|null} */let noOpReason=null;
 const check=()=>{if(asynchronous&&ctx.signal?.aborted)fail('cancelled',effect);};
 /** @param {import('./types.js').SizeResult['status']} status */
 const output=status=>Object.freeze({status,operation,targetRoute:route,effectState:effect,submitted:status==='size_submitted',windowChangeConfirmed:/** @type {const} */(false),
  originalTarget:original,selectedTarget:selected,className,plannedRect:planned,primaryScreen:primary,restoreRequested:restored,handoff,noOpReason});
 /** @param {string} method @param {boolean} write @param {()=>unknown} invoke @param {(value:unknown)=>SizeWork} next @param {boolean} [allowClassNoop] @returns {SizeWork} */
 const call=(method,write,invoke,next,allowClassNoop=false)=>{
  const reject=/** @param {unknown} cause @param {boolean} [fromInvocation] @returns {SizeWork} */(cause,fromInvocation=false)=>{
   const e=/** @type {{code?:unknown;message?:unknown;method?:unknown;effectState?:unknown}} */(cause);
   // 只允许初始geometry只读阶段的专用结构化拒绝转为来源无操作。
   // 不匹配消息正文，不吞unsupported_target/身份/期限/Native错误，也不接管restore之后的拒绝。
   if(fromInvocation&&allowClassNoop&&!write&&method==='window.geometry'&&e?.code==='window_class_blocked'
      &&e?.method==='window.geometry'&&e?.effectState==='not_started'){
    noOpReason='exact_platform_class';return output('source_class_blocked');
   }
   if(write&&e?.effectState!=='not_started'||e?.effectState==='unknown')effect='unknown';
   throw Object.assign(Error(typeof e?.message==='string'?e.message:'窗口大小调用未确认。'),{code:typeof e?.code==='string'&&/^[a-z_]{1,64}$/.test(e.code)?e.code:'window_step_failed',
    method:typeof e?.method==='string'?e.method:method,effectState:effect});
  };
  const accept=/** @param {unknown} value @returns {SizeContinuation} */value=>{
   if(write)effect='unknown';check();
   // 同步回执先退出本次调用帧，再推进后续步骤，避免将双读/交接压在同一栈上。
   return Object.freeze({resume:()=>{try{check();return next(value);}catch(cause){return reject(cause);}}});
  };
  try{
   check();let value;
   try{value=invoke();}catch(cause){return reject(cause,true);}
   if(value&&typeof /** @type {{then?:unknown}} */(value).then==='function'){
    if(!asynchronous){Promise.resolve(value).catch(()=>{});fail('runtime_contract_mismatch',write?'unknown':effect);}
    return Promise.resolve(value).then(accept,cause=>reject(cause,true)).catch(reject);
   }
   return accept(value);
  }catch(e){return reject(e);}
 };
 /** @param {T|null} target */
 const geometry=target=>target===null?d.getWindowGeometry():d.getWindowGeometryFor(target);
 /** @param {T|null} target @param {(g:import('./sdk/api.js').WindowGeometry)=>SizeWork} next @param {boolean} [allowClassNoop] */
 const doubleGeometry=(target,next,allowClassNoop=false)=>call('window.geometry',false,()=>geometry(target),raw=>{
  const first=decodeGeometry(raw);
  return call('window.geometry',false,()=>geometry(target),second=>{
   if(JSON.stringify(first)!==JSON.stringify(decodeGeometry(second)))fail('observation_changed',effect);
   return next(first);
  },allowClassNoop);
 },allowClassNoop);
 /** @param {T|null} target @returns {SizeWork} */
 const adjust=target=>doubleGeometry(target,before=>{
  className=before.className;primary=before.primaryScreen;
  if(sourceClassBlocked(className)){noOpReason='source_pattern';return output('source_class_blocked');}
  const plan=planSourceSizeStep({rect:before.visibleFrameRect,primaryScreen:before.primaryScreen,operation,step:input.step});
  if(plan.dimensionGaps.length)fail('source_size_semantics_unavailable',effect);
  planned=plan.sdkRect;
  if(planned===null)fail('invalid_response',effect);
  const frozenRect=planned;
  const submit=()=>call('window.set_rect',true,()=>target===null?d.setWindowRect(frozenRect,{frame:'visible'}):d.setWindowRectFor(target,frozenRect,{frame:'visible'}),written=>{
   const r=row(written,['submitted','effectState']);if(r.submitted!==true||r.effectState!=='unknown')fail('invalid_response','unknown');
   return output('size_submitted');
  });
  if(!before.maximized)return submit();
  restored=true;
  return call('window.action',true,()=>target===null?d.windowAction('restore'):d.restoreWindowFor(target),receipt=>{
   if(row(receipt,['submitted']).submitted!==true)fail('invalid_response','unknown');
   // 只确认还原状态和类型；原始±计算、位置及主屏数据均保持，不用新尺寸重算。
   return doubleGeometry(target,after=>{
    if(after.className!==className)fail('observation_changed','unknown');
    return after.maximized?output('restore_pending'):submit();
   });
  });
 },true);
 /** @param {SizeWork} work @returns {import('./types.js').Maybe<import('./types.js').SizeResult>} */
 const drain=work=>{
  while('resume' in work)work=work.resume();
  return 'then' in work?Promise.resolve(work).then(drain):work;
 };
 return drain(call('target.context',false,()=>d.targetContext(),a=>{
  original=decodeOriginal(a);selected=original;
  return call('pointer.root',false,()=>d.getPointingRoot(),p=>{
   const first=decodePointing(p);
   return call('pointer.root',false,()=>d.getPointingRoot(),p2=>{
    if(JSON.stringify(first)!==JSON.stringify(decodePointing(p2)))fail('observation_changed');
    return call('target.context',false,()=>d.targetContext(),a2=>{
     if(!same(original,decodeOriginal(a2)))fail('target_changed');
     if(same(original,first.rootWindow))return adjust(null);
     if(input.targetPolicy==='original-only')return output('pointing_target_differs');
     if(input.handoff===null)fail('invalid_input');
     const options=input.handoff;
     return call('target.handoff_pointing_root',true,()=>d.handoffPointingRoot(first,options),raw=>{
      const r=row(raw,['requested','requestAccepted','foregroundConfirmed','target','reason','effectState']);
      if(r.requested!==true||typeof r.requestAccepted!=='boolean'||typeof r.foregroundConfirmed!=='boolean'||r.effectState!=='unknown')fail('invalid_response','unknown');
      if(!r.requestAccepted){if(r.reason!=='os_rejected'||r.target!==null)fail('invalid_response','unknown');}
      else if(!r.foregroundConfirmed){if(r.reason!=='confirmation_timeout'||r.target!==null)fail('invalid_response','unknown');}
      else if(r.reason!==null||r.target===null)fail('invalid_response','unknown');
      handoff=Object.freeze({requestAccepted:r.requestAccepted,foregroundConfirmed:r.foregroundConfirmed,reason:r.reason});
      if(!r.requestAccepted||!r.foregroundConfirmed||r.target===null)return output('handoff_not_confirmed');
      const target=/** @type {T} */(r.target);
      if(!same(first.rootWindow,decodeOriginal(target.context)))fail('target_changed','unknown');
      selected=first.rootWindow;route='captured';return adjust(target);
     });
    });
   });
  });
 }));
}
