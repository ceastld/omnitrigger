const message='喝水提醒未获完整确认，请查看错误代码。';
/** @param {unknown} input @returns {import('./sdk/api').NotifyOptions} */
export function prepare(input){
  if(!input||typeof input!=='object'||Array.isArray(input)||Reflect.ownKeys(input).length!==0)throw Object.assign(new Error(message),{code:'invalid_input',effectState:'not_started'});
  return {message:'整点啦，起来喝杯水吧 💧',title:'喝水提醒',placement:'topRight',level:'info'};
}
/** @param {Pick<import('./sdk/api').ActionContext, 'ui'> | Pick<ScriptContext, 'ui'>} ctx */
export function available(ctx){
  if(typeof ctx.ui?.notify!=='function')throw Object.assign(new Error(message),{code:'ui_unavailable',effectState:'not_started'});
}
/** @param {unknown} receipt */
export function complete(receipt){
  try{
    if(!receipt||typeof receipt!=='object'||Array.isArray(receipt)||Reflect.ownKeys(receipt).length!==1||!Object.hasOwn(receipt,'submitted')||/** @type {{submitted?:unknown}} */(receipt).submitted!==true)throw new Error();
    return Object.freeze({submitted:true});
  }catch(_){throw Object.assign(new Error(message),{code:'invalid_response',effectState:'unknown'});}
}
/** @param {unknown} error @param {string} method @param {boolean} began @param {boolean} returned */
export function sanitized(error,method,began,returned=false){
  let code='notification_failed',effectState=began?'unknown':'not_started';
  try{
    if(typeof /** @type {{code?:unknown}|null|undefined} */(error)?.code==='string'&&/^[A-Za-z0-9_]{1,64}$/.test(/** @type {{code:string}} */(error).code))code=/** @type {{code:string}} */(error).code;
    if(began&&!returned&&/** @type {{effectState?:unknown}|null|undefined} */(error)?.effectState==='not_started')effectState='not_started';
  }catch(_){/* 不回显异常正文、权限或其他宿主字段，不重试。 */}
  return Object.assign(new Error(message),{code,method,effectState});
}
