const input={profile:'own',windowId:4,tabId:7};
const known={tabId:9,windowId:4};
export const cases=[
  {name:'默认复制',input,status:'completed',target:known,steps:1},
  {name:'显式聚焦',input:{...input,focusWindow:true},status:'completed',target:known,steps:2},
  {name:'只一连接可省profile',input:{windowId:4,tabId:7},status:'completed',target:known,steps:1},
  {name:'明确profile不默选第一项',input,profiles:[{browser_instance_id:'other'},{browser_instance_id:'own'}],status:'completed',target:known,steps:1},
  {name:'零标签ID',input:{...input,tabId:0},status:'completed',target:known,steps:1},
  {name:'零窗口ID',input:{...input,windowId:0},status:'completed',target:{tabId:9,windowId:0},steps:1},
  {name:'int32上界及副本0',input:{...input,windowId:2147483647,tabId:2147483647,focusWindow:true},copy:{id:0,windowId:2147483647},status:'completed',target:{tabId:0,windowId:2147483647},steps:2},
  ...[null,undefined,[],{}, {windowId:4}, {...input,windowId:-1},{...input,windowId:2147483648},
    {...input,tabId:'7'},{...input,tabId:2147483648},{...input,profile:''},{...input,focusWindow:'true'},
    {...input,activate:false},{...input,url:'https://invalid.example/'},{...input,timeoutMs:0},{...input,timeoutMs:60001},
    {...input,sessionId:'not-supported'}].map((input,i)=>({name:`参数拒绝${i}`,input,thrown:'invalid_input'})),
  {name:'指定profile不存在',input,profiles:[],thrown:'profile_unavailable'},
  {name:'无连接',input:{windowId:4,tabId:7},profiles:[],thrown:'browser_unavailable'},
  {name:'多个连接必须明确profile',input:{windowId:4,tabId:7},profiles:[{browser_instance_id:'own'},{browser_instance_id:'other'}],thrown:'ambiguous_profile'},
  {name:'重复profile身份',input,profiles:[{browser_instance_id:'own'},{browser_instance_id:'own'}],thrown:'invalid_response'},
  {name:'profile列表无效',input,profiles:null,thrown:'invalid_response'},
  ...[null,{}, {id:7,windowId:5},{id:8,windowId:4},{id:2147483648,windowId:4}].map((source,i)=>({name:`源身份拒绝${i}`,input,source,thrown:i===2||i===3?'target_changed':'invalid_response'})),
  ...[undefined,null,{}, {id:9},{id:2147483648,windowId:4},{id:9,windowId:2147483648}].map((copy,i)=>({name:`副本身份无效${i}`,input,copy,status:'partial',code:'invalid_response',steps:1})),
  {name:'副本异窗保留真实ID并停止聚焦',input:{...input,focusWindow:true},copy:{id:9,windowId:5},status:'partial',code:'target_changed',steps:1},
  ...[0,7,2147483647].flatMap(tabId=>[false,true].map(focusWindow=>({name:`副本仍为源ID${tabId}-focus${focusWindow}`,input:{...input,tabId,focusWindow},copy:{id:tabId,windowId:4},status:'partial',code:'invalid_response',steps:1}))),
  ...[undefined,{}, {id:5,focused:true},{id:4,focused:false},{id:4},{id:2147483648,focused:true}].map((focused,i)=>({name:`聚焦回执拒绝${i}`,input:{...input,focusWindow:true},focused,status:'partial',code:i===2?'target_changed':i===3||i===4?'focus_unconfirmed':'invalid_response',target:known,steps:2})),
  ...['profiles','tabs.get','tabs.duplicate','windows.update'].flatMap(at=>['not_started','unknown'].map(effect=>({
    name:`取消-${at}-${effect}`,input:{...input,focusWindow:at==='windows.update'},fault:{at,code:'cancelled',effect},
    ...(['profiles','tabs.get'].includes(at)?{thrown:'cancelled'}:{status:at==='windows.update'?'partial':effect==='not_started'?'failed':'unknown',code:'cancelled',steps:at==='windows.update'?1:0,...(at==='windows.update'?{target:known}:{})}),
  }))),
  {name:'复制权限拒绝',input,fault:{at:'tabs.duplicate',code:'permission_required',effect:'not_started',permissions:['tabs']},status:'failed',code:'permission_required',steps:0},
  {name:'聚焦权限拒绝保留副本',input:{...input,focusWindow:true},fault:{at:'windows.update',code:'permission_required',effect:'not_started',permissions:['windows']},status:'partial',code:'permission_required',target:known,steps:1},
  ...['profiles','tabs.get','tabs.duplicate'].map(mutateAt=>({name:`输入快照-${mutateAt}`,input:{...input,timeoutMs:1234},mutateAt,status:'completed',target:known,steps:1})),
  {name:'宿主改写options不污染后续',input:{...input,focusWindow:true,timeoutMs:1234},mutateOptions:true,status:'completed',target:known,steps:2},
];
const has=(obj,key)=>Object.prototype.hasOwnProperty.call(obj,key);
export function fixture(spec,sync){
  const raw=structuredClone(spec.input),original=structuredClone(spec.input),calls=[];
  const mutate=method=>{if(spec.mutateAt===method)Object.assign(raw,{profile:'other',windowId:99,tabId:98,focusWindow:true,timeoutMs:1});};
  const fault=method=>{if(spec.fault?.at===method)throw Object.assign(new Error('synthetic private detail'),{
    code:spec.fault.code,effectState:spec.fault.effect,...(spec.fault.permissions?{requiredPermissions:spec.fault.permissions}:{})});};
  const protect=value=>{
    if(value&&typeof value==='object')for(const key of ['url','title'])Object.defineProperty(value,key,{get(){throw Error('复制不应读取'+key);}});
    return value;
  };
  const profiles=options=>{calls.push({method:'profiles',options:structuredClone(options)});mutate('profiles');fault('profiles');
    if(spec.mutateOptions)Object.assign(options,{profile:'other',timeoutMs:1});
    return has(spec,'profiles')?structuredClone(spec.profiles):[{browser_instance_id:'own'}];};
  const call=(method,args,options)=>{
    calls.push(structuredClone({method,args,options}));mutate(method);fault(method);
    if(spec.mutateOptions)Object.assign(options,{profile:'other',timeoutMs:1});
    if(method==='tabs.get')return protect(has(spec,'source')?structuredClone(spec.source):{id:original.tabId,windowId:original.windowId});
    if(method==='tabs.duplicate')return protect(has(spec,'copy')?structuredClone(spec.copy):{id:9,windowId:original.windowId,active:false});
    if(method==='windows.update')return has(spec,'focused')?structuredClone(spec.focused):{id:original.windowId,focused:true};
    throw Error('禁止额外浏览器方法 '+method);
  };
  const ctx={input:raw,browser:sync?{profiles,call}:{profiles:async options=>profiles(options),call:async(...args)=>call(...args)}};
  for(const key of ['desktop','clipboard','process','fs','open'])Object.defineProperty(ctx,key,{get(){throw Error('禁止访问'+key);}});
  return{ctx,calls,original};
}
export async function observe(entry,spec,sync){
  const f=fixture(spec,sync);let result,error;
  try{const value=entry(f.ctx);if(sync&&value&&typeof value.then==='function')throw Error('同步入口返回Promise');
    if(!sync&&(!value||typeof value.then!=='function'))throw Error('Node入口未返回Promise');result=sync?value:await value;
  }catch(e){error={code:e.code,method:e.method,effectState:e.effectState,...(e.requiredPermissions?{requiredPermissions:e.requiredPermissions}:{})};}
  return{...(error?{error}:{result}),calls:f.calls,inputAfter:f.ctx.input};
}
