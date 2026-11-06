const own = (object,key) => Object.prototype.hasOwnProperty.call(object,key);
const record = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const fail = code => ({status:'rejected',errorCode:code,backend:'rust',submitted:false,commandEffectState:'not_started',retryAllowed:false});
const uncertain = (code,submitted=null,status='unknown') => ({status,errorCode:code,backend:'rust',submitted,commandEffectState:'unknown',retryAllowed:false});
function pathKey(value) {
  if(typeof value!=='string'||value.includes('\0'))return null;
  let path=value.replaceAll('\\','/');
  if(path.startsWith('//?/'))path=path.slice(4);
  if(!/^[A-Za-z]:\//.test(path))return null;
  const parts=path.slice(3).split('/');
  if(parts.includes('..'))return null;
  return path[0].toLowerCase()+':/'+parts.filter(part=>part!==''&&part!=='.').join('/').toLowerCase();
}
function local(value) { return typeof value === 'string' && /^[A-Za-z]:[\\/]/.test(value) && !value.includes('\0')
  && !value.split(/[\\/]/).some(part => part==='..' || /^(onedrive|icloud)/i.test(part) || /^dropbox$/i.test(part)); }
function inputs(ctx) {
  const value=ctx.input;
  const fields=['cliExecutable','hostExecutable','dataRoot','profile','timeoutMs','actionParameter'];
  if(!record(value)||!fields.every(key=>own(value,key))||Reflect.ownKeys(value).length!==fields.length)
    return {error:fail('explicit_rust_inputs_required')};
  if(!local(value.cliExecutable)||!local(value.hostExecutable)||!local(value.dataRoot)
      ||!value.cliExecutable.toLowerCase().endsWith('eaclip-cli.exe')
      ||!['product','script-test'].includes(value.profile)||!Number.isInteger(value.timeoutMs)
      ||value.timeoutMs<1||value.timeoutMs>59000) return {error:fail('invalid_rust_inputs')};
  const p=value.actionParameter;
  if(!record(p)||p.present!==true||!own(p,'value')||Reflect.ownKeys(p).length!==2
      ||p.value!==null&&typeof p.value!=='string') return {error:fail('explicit_action_parameter_required')};
  const command=p.value===null||p.value===''?'clip toggle':p.value;
  if(!command.trim()||command.includes('\0')||command.length>8192) return {error:fail('unsupported_command_line')};
  return {value,command};
}

/** Only an explicit load/preparation call may create this independent host. */
function invokeExplicitTestHost(ctx) {
  const parsed=inputs(ctx);if(parsed.error)return parsed.error;
  const value=parsed.value;
  if(value.profile!=='script-test'||typeof ctx.process?.start!=='function')return fail('explicit_test_start_required');
  const result=ctx.process.start(value.hostExecutable,['--background','--script-test-instance','--data-root',value.dataRoot,'--api-port','0']);
  if(!record(result)||!Number.isInteger(result.pid)||result.pid<1||result.pid>0xffffffff)
    return uncertain('invalid_host_start_receipt');
  return {status:'started',pid:result.pid,readyObserved:false,backend:'rust',businessDispatched:0};
}

/** Uses the new explicit-target/root RPC, not a foreground-only replacement. */
function invokeBound(ctx) {
  const parsed=inputs(ctx);if(parsed.error)return parsed.error;
  const {value,command}=parsed;
  if(typeof ctx.desktop?.targetContext!=='function'||typeof ctx.process?.capture!=='function')return fail('required_host_api_missing');
  let target;try{target=ctx.desktop.targetContext();}catch{return fail('target_context_unavailable');}
  if(!record(target)||typeof target.window_handle!=='string'||typeof target.process_created_file_time!=='string'
      ||!Number.isSafeInteger(target.expires_at_unix_ms))return fail('invalid_target_context');
  const remaining=target.expires_at_unix_ms-Date.now();
  if(remaining<=0||remaining>30000)return fail('target_expired_or_invalid');
  const budget=Math.min(value.timeoutMs,remaining);
  const args=['verb',command,'--json','--host',value.hostExecutable,'--no-start',
    value.profile==='script-test'?'--test-data-root':'--expected-data-root',value.dataRoot,
    '--target-context',JSON.stringify(target),'--timeout-ms',String(budget)];
  let captured;
  try{captured=ctx.process.capture(value.cliExecutable,args,{timeoutMs:Math.min(60000,budget+1000),allowNonZero:true});}
  catch(error){try{if(error?.effectState==='not_started')return fail('cli_not_started');}catch{}return uncertain('cli_result_unconfirmed');}
  let reply;
  try{
    if(!captured||!Number.isInteger(captured.code)||typeof captured.stdout!=='string'||typeof captured.stderr!=='string'
        ||captured.stderr.trim()!=='')return uncertain('invalid_cli_capture');
    reply=JSON.parse(captured.stdout);
  }catch{return uncertain('invalid_cli_reply');}
  if(!record(reply)||typeof reply.ok!=='boolean')return uncertain('invalid_cli_reply');
  const submitted=typeof reply.submitted==='boolean'?reply.submitted:null;
  if(reply.state==='partial'||reply.result?.status==='partial'||reply.result?.commandEffectState==='partial')
    return uncertain('cli_partial',submitted,'partial');
  if(reply.result?.status==='unknown'||reply.result?.commandEffectState==='unknown')
    return uncertain('cli_unconfirmed',submitted);
  if(reply.ok){
    const result=reply.result;
    if(captured.code!==0||reply.state!=='accepted'||!record(result)||result.started!==false
        ||result.targetContextVersion!==1||result.targetContextVerified!==true
        ||typeof result.executable!=='string'||pathKey(result.executable)!==pathKey(value.hostExecutable)
        ||typeof result.dataRoot!=='string'||pathKey(result.dataRoot)!==pathKey(value.dataRoot)
        ||value.profile==='script-test'&&(result.testProfile!==true||result.captureEnabled!==false))
      return uncertain('invalid_bound_acceptance',submitted);
    if(own(reply,'submitted')&&reply.submitted!==true)return uncertain('inconsistent_cli_acceptance',submitted);
    return {status:'accepted',backend:'rust',submitted:true,commandEffectState:'accepted_only',targetContextVerified:true,
      dataRootVerified:true,uiRendered:false,hostStartedByBusinessCall:false,retryAllowed:false};
  }
  if(captured.code===0||typeof reply.submitted!=='boolean'||!record(reply.error)||typeof reply.error.code!=='string')
    return uncertain('invalid_cli_failure',submitted);
  if(reply.state!==(reply.submitted?'unknown':'failed'))return uncertain('inconsistent_cli_failure',reply.submitted);
  return {status:reply.submitted?'unknown':'failed',backend:'rust',errorCode:reply.error.code,
    submitted:reply.submitted,commandEffectState:reply.submitted?'unknown':'not_started',retryAllowed:false};
}

/** Preserve submission evidence through the existing engine's fixed fault fields.
 * Extra Error properties support script catch; the bounded message also survives
 * FAULT_FIELDS -> ActionError, whose schema has no submitted/partial fields.
 */
function throwOutcome(result,phase) {
  const submitted=typeof result?.submitted==='boolean'?result.submitted:null;
  const notStarted=submitted===false&&result?.commandEffectState==='not_started'
    &&['rejected','failed'].includes(result?.status);
  const partial=result?.status==='partial';
  const submissionCode=submitted===true?'submitted':submitted===false?'not_submitted':'submission_unknown';
  const code=notStarted?'n10_not_started':`${partial?'n10_partial':'n10_unconfirmed'}_${submissionCode}`;
  const effectState=notStarted?'not_started':'unknown';
  const detail={schema_version:1,backend:'rust',phase,
    status:partial?'partial':notStarted?'not_started':'unknown',submitted,
    submissionState:submitted===true?'submitted':submitted===false?'not_submitted':'unknown',
    commandEffectState:effectState,retryAllowed:false};
  if(typeof result?.errorCode==='string'&&/^[A-Za-z0-9_.-]{1,64}$/.test(result.errorCode))detail.causeCode=result.errorCode;
  const message=(phase==='prepare'?(notStarted?'N10 宿主准备未启动。':'N10 宿主准备结果未确认；不自动重试。')
    :notStarted?'N10 命令未启动。':partial?'N10 操作结果仅部分确认；不自动重试。':'N10 执行结果未确认；不自动重试。')
    +'\nN10_DETAIL='+JSON.stringify(detail);
  throw Object.assign(new Error(message),{code,effectState,method:phase==='prepare'?'process.start':'cliphost.runVerbWithTargetV1',
    submitted,submissionState:detail.submissionState,commandEffectState:effectState,retryAllowed:false,
    status:detail.status,phase});
}

/** Main never returns rejected/unknown/partial as an ordinary script success. */
export function main(ctx) {
  let result;
  try{result=invokeBound(ctx);}catch{return throwOutcome(uncertain('adapter_exception'),'business');}
  if(result.status==='accepted')return result;
  return throwOutcome(result,'business');
}

/** Preparation stays separate; "started" is not host readiness or business acceptance. */
export function startExplicitTestHost(ctx) {
  let result;
  try{result=invokeExplicitTestHost(ctx);}catch(error){
    let notStarted=false;try{notStarted=error?.effectState==='not_started';}catch{}
    return throwOutcome(notStarted?fail('host_not_started'):uncertain('host_start_unconfirmed'),'prepare');
  }
  if(result.status==='started')return result;
  return throwOutcome(result,'prepare');
}
