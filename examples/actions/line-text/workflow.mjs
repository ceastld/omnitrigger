import {transform} from './transform.mjs';
const fail=code=>{throw Object.assign(new Error('文本行操作未完成，请查看错误代码。'),{code});};
export function prepare(input){
  if(!input||typeof input!=='object'||Array.isArray(input)||Reflect.ownKeys(input).some(key=>!['mode','operation','text'].includes(key)))fail('invalid_input');
  const {mode,operation,text}=input;
  if(!['inspect','send'].includes(mode))fail('invalid_input');
  return{mode,...transform(text,operation)};
}
export function done(task){
  if(task.mode==='inspect')return{status:'inspected',...task.summary,inputSubmitted:false};
  if(task.text==='')return{status:'empty',...task.summary,inputSubmitted:false};
  return null;
}
export function send(ctx,task){
  if(typeof ctx.desktop?.sendText!=='function')fail('desktop_unavailable');
  return ctx.desktop.sendText(task.text);
}
export function complete(task,receipt){
  if(!receipt||typeof receipt!=='object'||Array.isArray(receipt)||Reflect.ownKeys(receipt).length!==1||receipt.submitted!==true)fail('invalid_response');
  return{status:'submitted',...task.summary,inputSubmitted:true};
}
export function sanitized(error,method){
  let code='line_text_failed';let effectState=method==='script.parameters'?'not_started':'unknown';
  try{
    if(['invalid_input','input_too_large','output_too_large','too_many_lines','desktop_unavailable','invalid_response','cancelled','timeout','target_changed','target_not_foreground','input_failed'].includes(error?.code))code=error.code;
    if(error?.effectState==='not_started')effectState='not_started';
    if(code==='desktop_unavailable')effectState='not_started';
  }catch(_){/* 固定消息，不回显异常正文。 */}
  return Object.assign(new Error('文本行操作未完成，请查看错误代码。'),{code,method,effectState});
}
