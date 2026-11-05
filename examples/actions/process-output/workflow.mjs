// 显式自有程序 -> stdout通知；不读取脚本文件、不记录输出、不补发输入。
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const fail = code => {throw Object.assign(new Error('程序输出通知参数或结果无效。'),{code});};
function unicode(value,nul=false) {
  if(typeof value!=='string'||(!nul&&value.includes('\0')))return false;
  for(let index=0;index<value.length;index++){
    const unit=value.charCodeAt(index);
    if(unit>=0xd800&&unit<=0xdbff){const next=value.charCodeAt(++index);if(!(next>=0xdc00&&next<=0xdfff))return false;}
    else if(unit>=0xdc00&&unit<=0xdfff)return false;
  }
  return true;
}
const local = value => unicode(value)&&/^[A-Za-z]:[\\/]/.test(value)
  &&!value.slice(3).split(/[\\/]/).some(part=>part==='.'||part==='..');

export function prepare(input,capture) {
  const allowed=['program','args','cwd','allowNonZero','trimOutput',...(capture?['timeoutMs']:[])];
  if(!object(input)||Reflect.ownKeys(input).some(key=>!allowed.includes(key)))fail('invalid_input');
  const {program,args=[],cwd,allowNonZero=false,trimOutput=true,timeoutMs=10000}=input;
  if(typeof program!=='string'||program.length>32766||!local(program)||!program.toLowerCase().endsWith('.exe')
      ||!local(cwd)||!Array.isArray(args)||args.length>128||Reflect.ownKeys(args).length!==args.length+1
      ||typeof allowNonZero!=='boolean'||typeof trimOutput!=='boolean'
      ||!Number.isInteger(timeoutMs)||timeoutMs<1||timeoutMs>60000
      ||Object.keys(input).some(key=>input[key]===undefined))fail('invalid_input');
  for(let index=0;index<args.length;index++)if(!Object.hasOwn(args,index)||!unicode(args[index])||args[index].length>32766)fail('invalid_input');
  const quoted=value=>{
    const parts=['"'];let slashes=0;
    for(const character of value){
      if(character==='\\'){slashes++;continue;}
      parts.push('\\'.repeat(character==='"'?slashes*2+1:slashes),character);slashes=0;
    }
    parts.push('\\'.repeat(slashes*2),'"');return parts.join('');
  };
  if([program,...args].map(quoted).join(' ').length+1>32767)fail('invalid_input');
  return {program,args:[...args],cwd,allowNonZero,trimOutput,timeoutMs,capture};
}

export function available(ctx,task) {
  if(typeof ctx.ui?.notify!=='function')fail('ui_unavailable');
  if(typeof ctx.process?.[task.capture?'capture':'run']!=='function')fail(task.capture?'process_capture_unavailable':'process_unavailable');
}

export function execute(ctx,task) {
  const options={cwd:task.cwd,allowNonZero:task.allowNonZero,...(task.capture?{timeoutMs:task.timeoutMs}:{})};
  return ctx.process[task.capture?'capture':'run'](task.program,task.args,options);
}

export function notification(value,task) {
  if(!object(value)||Reflect.ownKeys(value).length!==3||!['code','stdout','stderr'].every(key=>Object.hasOwn(value,key))
      ||!Number.isInteger(value.code)||value.code< -2147483648||value.code>2147483647
      ||!unicode(value.stdout,true)||!unicode(value.stderr,true))fail('invalid_response');
  let bytes=0;
  for(const text of [value.stdout,value.stderr])for(const character of text){
    const point=character.codePointAt(0);bytes+=point<0x80?1:point<0x800?2:point<0x10000?3:4;
    if(bytes>65536)fail('process_output_too_large');
  }
  if(value.code!==0&&!task.allowNonZero)fail('process_exit_nonzero');
  const output=task.trimOutput?value.stdout.trim():value.stdout;
  const hasText=/[^\u0009-\u000d\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]/.test(output);
  const message=hasText?output:'程序已完成，没有标准输出。';
  if(message.length>1024||/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/.test(message))fail('output_not_displayable');
  return {params:{message,title:'程序输出',level:value.code===0?'info':'warning',durationMs:3000},
    summary:{completed:true,exitCode:value.code,stdoutUtf16Units:value.stdout.length,stderrUtf16Units:value.stderr.length}};
}

export function complete(value,receipt) {
  if(!object(receipt)||Reflect.ownKeys(receipt).length!==1||receipt.submitted!==true)fail('invalid_response');
  return {...value.summary,notified:true};
}

export function sanitized(error,method,completed) {
  const known=['invalid_input','invalid_arguments','invalid_response','output_not_displayable','ui_unavailable','ui_busy','ui_error',
    'process_capture_unavailable','process_unavailable','process_setup_failed','process_start_failed','process_wait_failed','process_read_failed',
    'process_timeout','process_output_too_large','process_invalid_utf8','process_exit_nonzero','process_cleanup_failed','process_clock_error',
    'unsupported_platform','process_failed','output_limit','cancelled','timeout'];
  let code='process_output_failed';let effectState=method==='script.parameters'?'not_started':'unknown';
  try{
    if(known.includes(error?.code))code=error.code;
    if(['not_started','unknown'].includes(error?.effectState))effectState=error.effectState;
    if(['invalid_arguments','process_capture_unavailable','process_unavailable','process_setup_failed','process_start_failed','unsupported_platform'].includes(code))effectState='not_started';
    if(code==='ui_unavailable'&&!completed)effectState='not_started';
  }catch(_){/* 不回显原始错误消息、程序参数或输出。 */}
  if(completed)effectState='unknown';
  return Object.assign(new Error('程序输出通知未完成，请查看错误代码。'),{code,method,effectState});
}
