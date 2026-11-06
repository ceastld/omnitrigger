const input={profile:'own',windowId:4};
export const cases=[
  {name:'默认激活',input}, {name:'后台',input:{...input,activate:false,focusWindow:false}},
  {name:'显式聚焦',input:{...input,focusWindow:true}}, {name:'自定超时',input:{...input,timeoutMs:1234}},
  {name:'零窗口ID',input:{...input,windowId:0},window:{id:0,type:'normal'},tab:{id:0,windowId:0,active:true}},
  {name:'int32上界',input:{...input,windowId:2147483647},window:{id:2147483647,type:'normal'},tab:{id:2147483647,windowId:2147483647,active:true}},
  {name:'窗口超int32',input:{...input,windowId:2147483648}},
  {name:'创建标签ID超int32',input,tab:{id:2147483648,windowId:4}},
  {name:'创建窗口ID超int32',input,tab:{id:7,windowId:2147483648}},
  ...[{},null,[],{windowId:4},{...input,url:'chrome://newtab/'},{...input,tabId:7},
    {...input,browser:'chrome'},{...input,windowId:-1},{...input,windowId:9007199254740992},
    {...input,activate:'false'},{...input,activate:false,focusWindow:true},{...input,timeoutMs:0},
    {...input,timeoutMs:60001},{...input,profile:'\n'}].map((value,i)=>({name:`参数拒绝${i}`,input:value})),
  {name:'profile缺失',input,profiles:[]},
  {name:'profile重复',input,profiles:[{browser_instance_id:'own'},{browser_instance_id:'own'}]},
  {name:'profile响应错误',input,profiles:null},
  {name:'profile含无效身份',input,profiles:[{browser_instance_id:''}]},
  ...[{id:99,type:'normal'},{id:4,type:'popup'},null].map((window,i)=>({name:`窗口拒绝${i}`,input,window})),
  ...[{},null,{id:7,windowId:99}].map((tab,i)=>({name:`创建身份异常${i}`,input,tab})),
  ...['profiles','windows.get','tabs.create','windows.update'].flatMap(at=>['not_started','unknown'].map(effect=>({
    name:`取消-${at}-${effect}`,input:{...input,focusWindow:at==='windows.update'},fault:{at,code:'cancelled',effect}}))),
  {name:'创建Chrome错误',input,fault:{at:'tabs.create',code:'chrome_error',effect:'unknown'}},
  {name:'创建权限错误',input,fault:{at:'tabs.create',code:'permission_required',effect:'not_started',permissions:['tabs']}},
  {name:'聚焦权限错误保留创建',input:{...input,focusWindow:true},fault:{at:'windows.update',code:'permission_required',effect:'not_started',permissions:['windows']}},
  {name:'profiles期间输入被改写',input,mutateAt:'profiles'},
  {name:'窗口读取期间输入被改写',input,mutateAt:'windows.get'},
  {name:'创建期间输入被改写',input,mutateAt:'tabs.create'},
  {name:'不读默认页URL',input,tabUrlGetter:true},
];
const copy=value=>value===undefined?undefined:JSON.parse(JSON.stringify(value));
const has=(value,key)=>Object.prototype.hasOwnProperty.call(value,key);
export function fixture(spec,asyncApi=false){
  const calls=[];const originalInput=copy(spec.input);
  const mutate=method=>{if(spec.mutateAt===method)Object.assign(originalInput,{profile:'other',windowId:99,focusWindow:true,activate:false});};
  const fault=method=>{if(spec.fault?.at===method)throw Object.assign(new Error('合成宿主错误'),{
    code:spec.fault.code,effectState:spec.fault.effect,
    ...(spec.fault.permissions?{requiredPermissions:copy(spec.fault.permissions)}:{})});};
  const profiles=options=>{calls.push({method:'profiles',options:copy(options)});mutate('profiles');fault('profiles');
    return copy(has(spec,'profiles')?spec.profiles:[{browser_instance_id:'own'}]);};
  const call=(method,args,options)=>{calls.push({method,args:copy(args),options:copy(options)});mutate(method);fault(method);
    if(method==='windows.get')return copy(has(spec,'window')?spec.window:{id:4,type:'normal'});
    if(method==='tabs.create'){
      const tab=copy(has(spec,'tab')?spec.tab:{id:7,windowId:4,active:args[0].active});
      if(spec.tabUrlGetter)Object.defineProperty(tab,'url',{get(){throw Error('禁止读取默认页URL');}});
      return tab;
    }
    if(method==='windows.update')return {id:args[0],focused:true};
    throw Error('禁止额外调用 '+method);
  };
  const browser=asyncApi?{profiles:async options=>profiles(options),call:async(...args)=>call(...args)}:{profiles,call};
  const ctx={input:originalInput,browser};
  for(const key of ['desktop','clipboard','process','open'])Object.defineProperty(ctx,key,{get(){throw Error('禁止访问 '+key);}});
  return {ctx,calls,input:originalInput};
}
function errorFields(error){
  return {name:error?.name,code:error?.code,message:error?.message,method:error?.method,effectState:error?.effectState,
    ...(error?.requiredPermissions?{requiredPermissions:error.requiredPermissions}:{})};
}
export function executeSync(spec,entry){
  const f=fixture(spec);let outcome;
  try{const value=entry(f.ctx);if(value&&typeof value.then==='function')throw Error('同步入口返回Promise');outcome={returned:value};}
  catch(error){outcome={thrown:errorFields(error)};}
  return copy({name:spec.name,...outcome,calls:f.calls,inputAfter:f.input});
}
export async function executeAsync(spec,entry){
  const f=fixture(spec,true);let outcome;
  try{outcome={returned:await entry(f.ctx)};}catch(error){outcome={thrown:errorFields(error)};}
  return copy({name:spec.name,...outcome,calls:f.calls,inputAfter:f.input});
}
