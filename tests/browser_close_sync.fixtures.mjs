export const base={profile:'owned',windowId:4,tabId:7};
const trace=['profiles','windows.get','tabs.get','tabs.remove'];
const invalid=(name,input)=>({name,input,expected:{kind:'throw',code:'invalid_input',trace:[]}});
export const scenarios=[
  {name:'正常窗口void确认',expected:{kind:'return',status:'completed',trace}},
  {name:'popup窗口确认',window:{id:4,type:'popup'},expected:{kind:'return',status:'completed',trace}},
  {name:'null确认回执',removeReply:null,expected:{kind:'return',status:'completed',trace}},
  {name:'显式数字0',input:{profile:'owned',windowId:0,tabId:0},expected:{kind:'return',status:'completed',trace}},
  {name:'int32上界',input:{profile:'owned',windowId:2147483647,tabId:2147483647},expected:{kind:'return',status:'completed',trace}},
  {name:'显式期限和多个profile',input:{...base,timeoutMs:1234},profiles:[{browser_instance_id:'other'},{browser_instance_id:'owned'}],expected:{kind:'return',status:'completed',trace}},
  {name:'没有连接',profiles:[],expected:{kind:'throw',code:'profile_unavailable',trace:['profiles']}},
  {name:'指定连接消失',profiles:[{browser_instance_id:'other'}],expected:{kind:'throw',code:'profile_unavailable',trace:['profiles']}},
  {name:'连接格式无效',profiles:null,expected:{kind:'throw',code:'invalid_response',trace:['profiles']}},
  {name:'连接ID缺失',profiles:[{}],expected:{kind:'throw',code:'invalid_response',trace:['profiles']}},
  {name:'连接ID重复',profiles:[{browser_instance_id:'owned'},{browser_instance_id:'owned'}],expected:{kind:'throw',code:'invalid_response',trace:['profiles']}},
  {name:'源窗口消失',window:null,expected:{kind:'throw',code:'target_changed',trace:trace.slice(0,2)}},
  {name:'窗口ID不同',window:{id:5,type:'normal'},expected:{kind:'throw',code:'target_changed',trace:trace.slice(0,2)}},
  {name:'devtools窗口',window:{id:4,type:'devtools'},expected:{kind:'throw',code:'target_changed',trace:trace.slice(0,2)}},
  {name:'标签消失',tab:null,expected:{kind:'throw',code:'target_changed',trace:trace.slice(0,3)}},
  {name:'标签ID不同',tab:{id:8,windowId:4},expected:{kind:'throw',code:'target_changed',trace:trace.slice(0,3)}},
  {name:'标签已移动',tab:{id:7,windowId:8},expected:{kind:'throw',code:'target_changed',trace:trace.slice(0,3)}},
  invalid('空输入',{}),invalid('数组输入',[]),invalid('缺profile',{windowId:4,tabId:7}),
  invalid('窗口超int32',{...base,windowId:2147483648}),invalid('标签超int32',{...base,tabId:Number.MAX_SAFE_INTEGER}),
  invalid('窗口小数',{...base,windowId:4.5}),invalid('标签字符串',{...base,tabId:'7'}),
  invalid('标签嵌套引用',{...base,tabId:{id:7}}),invalid('profile空白',{...base,profile:' '}),
  invalid('错误字段',{...base,active:true}),invalid('期限0',{...base,timeoutMs:0}),invalid('期限超限',{...base,timeoutMs:60001}),
  {name:'配置查询取消',fault:{method:'profiles',code:'cancelled',effectState:'not_started'},expected:{kind:'throw',code:'cancelled',trace:['profiles']}},
  {name:'窗口查询权限不足',fault:{method:'windows.get',code:'permission_required',effectState:'not_started'},expected:{kind:'throw',code:'permission_required',trace:trace.slice(0,2)}},
  {name:'标签查询断连',fault:{method:'tabs.get',code:'disconnected',effectState:'unknown'},expected:{kind:'throw',code:'disconnected',trace:trace.slice(0,3)}},
  ...[['not_started','failed','permission_required'],['not_started','failed','cancelled'],['partial','partial','partial_effect'],['unknown','unknown','disconnected'],[undefined,'unknown','timeout'],['invalid','unknown','unexpected']]
    .map(([effectState,status,code])=>({name:`关闭${code}/${effectState}`,fault:{method:'tabs.remove',code,effectState,requiredPermissions:['tabs']},expected:{kind:'return',status,trace}})),
  ...['profiles','windows.get','tabs.get','tabs.remove'].map(mutateAt=>({name:`${mutateAt}等待点改写原输入`,mutateAt,expected:{kind:'return',status:'completed',trace}})),
];

export function fakeContext(scenario,{sync=false}={}){
  const input=structuredClone(scenario.input??base),original=structuredClone(input),calls=[];
  const effects={removeAttempts:0,started:false};
  const ctx={input,browser:null};
  function dispatch(method,args,options){
    calls.push(structuredClone({method,args:args??null,options}));
    if(method===scenario.mutateAt)Object.assign(ctx.input,{profile:'other',windowId:40,tabId:70,timeoutMs:1});
    if(method==='tabs.remove')effects.removeAttempts++;
    if(method===scenario.fault?.method){
      if(method==='tabs.remove')effects.started=scenario.fault.effectState!=='not_started';
      throw Object.assign(new Error('合成私有错误正文'),{...scenario.fault,method:method==='profiles'?'browser.profiles':method});
    }
    switch(method){
      case 'profiles':return Object.hasOwn(scenario,'profiles')?scenario.profiles:[{browser_instance_id:'owned'}];
      case 'windows.get':return Object.hasOwn(scenario,'window')?scenario.window:{id:original.windowId,type:'normal'};
      case 'tabs.get':return Object.hasOwn(scenario,'tab')?scenario.tab:{id:original.tabId,windowId:original.windowId};
      case 'tabs.remove':effects.started=true;return scenario.removeReply;
      default:throw Error(`非预期API ${method}`);
    }
  }
  const browser={profiles:options=>dispatch('profiles',undefined,options),call:(method,args,options)=>dispatch(method,args,options)};
  ctx.browser=sync?browser:{profiles:async options=>browser.profiles(options),call:async(...args)=>browser.call(...args)};
  return{ctx,calls,effects,original};
}

export async function observe(main,scenario,{sync=false}={}){
  const f=fakeContext(scenario,{sync});
  try{
    const value=main(f.ctx);
    if(sync&&value&&typeof value.then==='function')throw Error('同步入口意外返回Promise');
    const result=sync?value:await value;
    return{kind:'return',result,calls:f.calls,effects:f.effects};
  }catch(error){
    return{kind:'throw',error:{code:error.code,method:error.method,effectState:error.effectState,message:error.message,
      ...(error.requiredPermissions?{requiredPermissions:error.requiredPermissions}:{})},calls:f.calls,effects:f.effects};
  }
}
