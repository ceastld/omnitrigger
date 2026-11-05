import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import closeTab from '../examples/actions/browser-close/close-tab.mjs';
import popOutTab from '../examples/actions/browser-popout/pop-out-tab.mjs';

const currentApiSource=await readFile(new URL('../crates/omnitrigger-actions/runtime/browser-api.js',import.meta.url),'utf8');
const createApi=new Function(`${currentApiSource};return createBrowserApi;`)();
const MAX=2147483647;
const main={close:closeTab,popout:popOutTab};
function fixture(kind,{input={profile:'owned',windowId:4,tabId:7},faultAt,fault,createdId=9,onCall}={}){
  const calls=[],world={closed:false,created:false};let reads=0;
  const sourceWindow={id:input.windowId,type:'normal',incognito:false};
  const created={id:createdId,type:'popup',incognito:false};
  const ctx={input:{...input},browser:null};
  const api=createApi(async(method,args,options,profiles)=>{
    calls.push(structuredClone({method:profiles?'profiles':method,args:args??null,options}));
    await onCall?.({ctx,method:profiles?'profiles':method,args,options,sourceWindow,created,reads});
    if(profiles)return[{browser_instance_id:'owned'}];
    if(method===faultAt&&fault?.effectState==='not_started')throw fault;
    if(method==='tabs.remove')world.closed=true;
    if(method==='windows.create')world.created=true;
    if(method===faultAt)throw fault;
    switch(method){
      case 'windows.get':return sourceWindow;
      case 'tabs.get':return{id:input.tabId,windowId:++reads===1?input.windowId:createdId,incognito:false};
      case 'tabs.remove':return undefined;
      case 'windows.create':return created;
      default:assert.fail(`unexpected ${method}`);
    }
  },fn=>async(...args)=>fn(...args));
  ctx.browser=api;return{ctx,calls,world,created,sourceWindow,kind};
}
const error=(effectState='unknown',extra={})=>Object.assign(new Error('private response body'),{code:'disconnected',effectState,...extra});
const mutations=f=>f.calls.filter(c=>['tabs.remove','windows.create'].includes(c.method));

for(const kind of Object.keys(main))for(const field of ['windowId','tabId']){
  test(`${kind} ${field} 超过Chrome int32范围在任何API前拒绝`,async()=>{
    for(const value of [MAX+1,4294967295,Number.MAX_SAFE_INTEGER,Number.MAX_SAFE_INTEGER+1]){
      const f=fixture(kind,{input:{profile:'owned',windowId:4,tabId:7,[field]:value}});
      await assert.rejects(main[kind](f.ctx),e=>e.code==='invalid_input'&&e.effectState==='not_started');
      assert.deepEqual(f.calls,[]);
    }
  });
}

for(const kind of Object.keys(main))test(`${kind} 非整数和嵌套标量不被强制转换或借用`,async()=>{
  for(const field of ['windowId','tabId'])for(const value of [1.5,NaN,Infinity,'7',null,{id:7},[7]]){
    const f=fixture(kind,{input:{profile:'owned',windowId:4,tabId:7,[field]:value}});
    await assert.rejects(main[kind](f.ctx),e=>e.code==='invalid_input');assert.deepEqual(f.calls,[]);
  }
  for(const extra of [{profile:{id:'owned'}},{timeoutMs:{value:1000}},{metadata:{names:['private']}}]){
    const f=fixture(kind,{input:{profile:'owned',windowId:4,tabId:7,...extra}});
    await assert.rejects(main[kind](f.ctx),e=>e.code==='invalid_input');assert.deepEqual(f.calls,[]);
  }
});

test('0和int32上界仍按明确ID核对，不解释为当前目标或进行截断',async()=>{
  for(const kind of Object.keys(main))for(const value of [0,MAX]){
    const f=fixture(kind,{input:{profile:'owned',windowId:value,tabId:value}});
    const result=await main[kind](f.ctx);assert.equal(result.status,'completed');
    const mutation=mutations(f);assert.equal(mutation.length,1);
    assert.equal(kind==='close'?mutation[0].args[0]:mutation[0].args[0].tabId,value);
  }
});

test('弹出API已确认但返回超大或非整数新窗口ID，保留partial且不追查猜测ID',async()=>{
  for(const createdId of [MAX+1,Number.MAX_SAFE_INTEGER,9.5]){
    const f=fixture('popout',{createdId});const result=await popOutTab(f.ctx);
    assert.equal(result.status,'partial');assert.equal(result.createdWindowId,undefined);assert.equal(result.target,undefined);
    assert.deepEqual(result.steps,[{method:'windows.create',tabId:7}]);
    assert.equal(result.error.code,'invalid_response');assert.equal(f.calls.length,4);
  }
});

for(const kind of Object.keys(main))test(`${kind} 已派发后断连是unknown，即使假对端已经产生效果也不重试`,async()=>{
  const f=fixture(kind,{faultAt:kind==='close'?'tabs.remove':'windows.create',fault:error()});
  const result=await main[kind](f.ctx);assert.equal(result.status,'unknown');assert.equal(result.error.effect_state,'unknown');
  assert.equal(mutations(f).length,1);assert.deepEqual(result.steps,[]);
  assert.equal(kind==='close'?f.world.closed:f.world.created,true);
  if(kind==='close')assert.equal(result.closeAcknowledged,false);
  else{assert.equal(result.createdWindowId,undefined);assert.equal(result.target,undefined);}
});

for(const kind of Object.keys(main))test(`${kind} 明确not_started与partial不混同，错误附带的嵌套结果不能冒充身份`,async()=>{
  for(const [effectState,status] of [['not_started','failed'],['partial','partial']]){
    const f=fixture(kind,{faultAt:kind==='close'?'tabs.remove':'windows.create',fault:error(effectState,{partialResult:{id:9,tabs:[{id:7,windowId:9}]}})});
    const result=await main[kind](f.ctx);assert.equal(result.status,status);assert.equal(result.error.effect_state,effectState);
    assert.equal(kind==='close'?f.world.closed:f.world.created,effectState!=='not_started');
    assert.equal(mutations(f).length,1);assert.deepEqual(result.steps,[]);assert.equal(result.createdWindowId,undefined);
    if(kind==='popout')assert.equal(result.target,undefined);
    assert.ok(!JSON.stringify(result).includes('partialResult'));
  }
});

test('创建确认后复查断连，保留已知新窗口和partial，不把未知复查当成没有创建',async()=>{
  const f=fixture('popout',{onCall:({method,reads})=>{if(method==='tabs.get'&&reads===1)throw error();}});
  const result=await popOutTab(f.ctx);assert.equal(result.status,'partial');assert.equal(result.createdWindowId,9);
  assert.deepEqual(result.steps,[{method:'windows.create',tabId:7,windowId:9}]);assert.equal(result.target,undefined);
  assert.equal(result.error.method,'tabs.get');assert.equal(result.error.effect_state,'unknown');assert.equal(mutations(f).length,1);
});

test('前置只读查询断连不派发两个动作的副作用，也不隐藏原错误',async()=>{
  for(const kind of Object.keys(main)){
    const fault=error();const f=fixture(kind,{faultAt:'windows.get',fault});
    await assert.rejects(main[kind](f.ctx),e=>e===fault);assert.equal(mutations(f).length,0);
  }
});

test('弹出等待复查期间原参数和已返回对象改写，不改变已固定目标及隐私模式',async()=>{
  const f=fixture('popout',{onCall:({ctx,method,sourceWindow,created,reads})=>{
    if(method==='tabs.get'&&reads===1){
      Object.assign(ctx.input,{profile:'other',windowId:40,tabId:70,focusWindow:true,timeoutMs:1});
      sourceWindow.incognito=true;created.id=100;created.incognito=true;
    }
  }});
  const result=await popOutTab(f.ctx);assert.equal(result.status,'completed');assert.equal(result.profile,'owned');
  assert.equal(result.createdWindowId,9);assert.deepEqual(result.target,{tabId:7,windowId:9});
  assert.ok(f.calls.filter(c=>c.method!=='profiles').every(c=>c.options.profile==='owned'&&c.options.timeoutMs===10000));
  assert.deepEqual(mutations(f)[0].args,[{tabId:7,type:'popup',focused:false,incognito:false}]);
});

for(const kind of Object.keys(main))test(`${kind} 错误权限数组被复制且非字符串嵌套项被丢弃`,async()=>{
  const nested={names:['private']},permissions=['tabs',nested];
  const fault=error('unknown',{requiredPermissions:permissions});
  const f=fixture(kind,{faultAt:kind==='close'?'tabs.remove':'windows.create',fault});
  const result=await main[kind](f.ctx);permissions[0]='sessions';permissions.push('bookmarks');nested.names[0]='changed';
  assert.deepEqual(result.error.required_permissions,['tabs']);assert.ok(!JSON.stringify(result).includes('private'));
});
