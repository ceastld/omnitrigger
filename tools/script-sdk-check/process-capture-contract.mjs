// 开发/验收用的记录后端契约，不启动进程，不是已接入运行时的 process.capture 实现。
// wire/回执来自现有 Rust Capture DTO；取消事件表示外层调用者取消，不声称原生循环支持回调抢占。
export const captureLimits = Object.freeze({bytes:65536, args:128, requestBytes:256*1024,
  timeoutDefault:10000, timeoutMaximum:60000});
const method='process.capture';
const maximumClock=(1n<<64n)-1n;
const errorCodes=new Set(['invalid_input','invalid_arguments','request_too_large','unsupported_platform','process_capture_failed','process_setup_failed','process_start_failed',
  'process_read_failed','process_wait_failed','process_clock_error','process_output_too_large',
  'process_timeout','process_invalid_utf8','process_exit_nonzero','process_cleanup_failed','cancelled','invalid_response']);
function failure(code,started) {
  return Object.assign(new Error('程序输出采集未完成，请检查错误代码。'),
    {code:errorCodes.has(code)?code:'process_capture_failed',method,effectState:started?'unknown':'not_started'});
}
function reportedCode(error) {try{return error?.code;}catch{return undefined;}}
function fields(value,required,optional=[]) {
  return value!==null&&typeof value==='object'&&!Array.isArray(value)
    &&required.every(key=>Object.hasOwn(value,key))
    &&Reflect.ownKeys(value).every(key=>required.includes(key)||optional.includes(key));
}
function unicode(value) {
  if(typeof value!=='string')return false;
  for(let index=0;index<value.length;index++) {
    const unit=value.charCodeAt(index);
    if(unit>=0xd800&&unit<=0xdbff) {
      const next=value.charCodeAt(++index);
      if(!(next>=0xdc00&&next<=0xdfff))return false;
    } else if(unit>=0xdc00&&unit<=0xdfff)return false;
  }
  return true;
}
function absolute(value) {
  return unicode(value)&&/^[A-Za-z]:[\\/]/.test(value)&&!value.includes('\0')
    &&!value.slice(3).split(/[\\/]/).some(part=>part==='.'||part==='..');
}
function quotedUnits(value) {
  let units=2,slashes=0;
  for(const point of value) {
    if(point==='\\'){slashes++;continue;}
    units+=slashes*(point==='"'?2:1)+(point==='"'?1:0)+point.length;
    slashes=0;
  }
  return units+slashes*2;
}
export function normalizeCaptureWire(request) {
  try {
    if(!fields(request,['kind','program'],['args','timeout_ms','allow_non_zero','cwd'])||request.kind!=='capture')throw Error();
    const {program,args=[],timeout_ms=10000,allow_non_zero=false,cwd=null}=request;
    if(!absolute(program)||!program.toLowerCase().endsWith('.exe')
      ||!Array.isArray(args)||args.length>128||Reflect.ownKeys(args).length!==args.length+1
      ||!Number.isInteger(timeout_ms)||timeout_ms<1||timeout_ms>60000||typeof allow_non_zero!=='boolean'
      ||(cwd!==null&&!absolute(cwd)))throw Error();
    const copied=[];
    for(let index=0;index<args.length;index++) {
      if(!Object.hasOwn(args,index)||!unicode(args[index])||args[index].includes('\0'))throw Error();
      copied.push(args[index]);
    }
    const commandUnits=[program,...copied].reduce((units,item)=>units+quotedUnits(item),copied.length+1);
    if(commandUnits>32767)throw Error();
    const result={kind:'capture',program,args:copied,timeout_ms,allow_non_zero,cwd};
    if(Buffer.byteLength(JSON.stringify(result),'utf8')>captureLimits.requestBytes)throw failure('request_too_large',false);
    return result;
  } catch(error) {throw failure(reportedCode(error)==='request_too_large'?'request_too_large':'invalid_arguments',false);}
}
export function validateCapturedValue(value) {
  try {
    if(!fields(value,['code','stdout','stderr']))throw Error();
    const {code,stdout,stderr}=value;
    if(!Number.isInteger(code)||code<-2147483648||code>2147483647||!unicode(stdout)||!unicode(stderr)
      ||Buffer.byteLength(stdout,'utf8')+Buffer.byteLength(stderr,'utf8')>captureLimits.bytes)throw Error();
    return {code,stdout,stderr};
  } catch {throw failure('invalid_response',true);}
}
function decode(parts) {
  try {
    // ignoreBOM=true 保留真实U+FEFF；String::from_utf8同样不会删除BOM。
    return new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(Buffer.concat(parts));
  } catch {throw failure('process_invalid_utf8',true);}
}

export class RecordingCaptureBackend {
  constructor(events,{clock=0n,startError,cleanupError,privateJobEmpty=true,parentReaped=true,cancelBefore=false}={}) {
    this.events=events;this.clock=clock;this.startError=startError;this.cleanupError=cleanupError;
    this.privateJobEmpty=privateJobEmpty;this.parentReaped=parentReaped;this.cancelBefore=cancelBefore;
    this.records=[];
  }
  start(request) {
    this.records.push({kind:'start',program:request.program,args:request.args.slice(),options:{timeout_ms:request.timeout_ms,allow_non_zero:request.allow_non_zero,cwd:request.cwd}});
    if(this.startError)throw failure(this.startError,false);
  }
  finish() {
    this.records.push({kind:'finish',scope:'private-job'});
    if(this.cleanupError||!this.privateJobEmpty||!this.parentReaped)throw failure('process_cleanup_failed',true);
  }
}

export function runRecordedCapture(request,backend) {
  const normalized=normalizeCaptureWire(request);
  if(backend.cancelBefore)throw failure('cancelled',false);
  const began=backend.clock;
  if(typeof began!=='bigint'||began<0n||began>maximumClock||began+BigInt(normalized.timeout_ms)>maximumClock)
    throw failure('process_clock_error',false);
  const deadline=began+BigInt(normalized.timeout_ms);
  try {backend.start(normalized);}
  catch(error) {
    let knownBefore=false;
    try {knownBefore=error?.effectState==='not_started';}catch { /* 未知阶段保守处理。 */ }
    if(!knownBefore)try {backend.finish();}catch {throw failure('process_cleanup_failed',true);}
    throw failure(reportedCode(error),!knownBefore);
  }
  let result,error;
  try {
    const parts={stdout:[],stderr:[]};
    const eof={stdout:false,stderr:false};
    let bytes=0,last=began;
    for(const event of backend.events) {
      if(event.type==='stdout'||event.type==='stderr') {
        if(eof[event.type]||!(event.bytes instanceof Uint8Array)||typeof event.eof!=='boolean')throw failure('process_read_failed',true);
        bytes+=event.bytes.byteLength;
        if(bytes>captureLimits.bytes)throw failure('process_output_too_large',true);
        parts[event.type].push(Buffer.from(event.bytes));eof[event.type]=event.eof;
      } else if(event.type==='poll') {
        if(typeof event.now!=='bigint'||event.now<last||event.now>maximumClock)throw failure('process_clock_error',true);
        last=event.now;
        if(last>=deadline)throw failure('process_timeout',true);
        if(event.code!==null) {
          if(!Number.isInteger(event.code)||event.code<-2147483648||event.code>2147483647)throw failure('process_wait_failed',true);
          if(eof.stdout&&eof.stderr) {
            const stdout=decode(parts.stdout),stderr=decode(parts.stderr);
            if(event.code!==0&&!normalized.allow_non_zero)throw failure('process_exit_nonzero',true);
            result={code:event.code,stdout,stderr};break;
          }
        }
      } else if(event.type==='cancel')throw failure('cancelled',true);
      else if(event.type==='failure')throw failure(event.code,true);
      else throw failure('process_read_failed',true);
    }
    if(!result)throw failure('process_read_failed',true);
  } catch(cause) {error=failure(reportedCode(cause),true);}
  // finish是私有Job范围，不把直接子进程kill/close当作全部后代已收尾。
  try {backend.finish();}catch {throw failure('process_cleanup_failed',true);}
  if(error)throw error;
  return validateCapturedValue(result);
}
export function recordedCaptureAdapter(asynchronous,backend) {
  return request=>asynchronous?Promise.resolve().then(()=>runRecordedCapture(request,backend)):runRecordedCapture(request,backend);
}
