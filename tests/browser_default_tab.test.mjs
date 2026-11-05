// 纯RecordingHost回归；不创建浏览器、窗口、子进程或真实连接器。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import {runWorkflow} from '../examples/actions/browser-workflows/workflows.mjs';
const root=new URL('../',import.meta.url);
const base={profile:'synthetic-profile',windowId:4};
const window={id:4,type:'normal'};const created={id:7,windowId:4};
const fault=(code,effectState='unknown')=>Object.assign(new Error('合成失败'),{code,effectState});
function fixture(input=base,replies=[window,created],profiles=[{browser_instance_id:'synthetic-profile'}],onProfiles){
  const calls=[];
  return{input,calls,browser:{
    async profiles(options){calls.push({method:'profiles',options});onProfiles?.();return profiles;},
    async call(method,args,options){calls.push({method,args,options});assert.ok(replies.length,'禁止额外调用 '+method);
      const result=replies.shift();if(result instanceof Error)throw result;return result;},
  }};
}
const names=ctx=>ctx.calls.map(call=>call.method);
const reject=(ctx,code)=>assert.rejects(runWorkflow(ctx,'create-default-tab'),error=>error.code===code);

test('创建默认新标签只调用现有API，url字段完全省略',async()=>{
  const ctx=fixture();const result=await runWorkflow(ctx,'create-default-tab');
  assert.equal(result.status,'completed');assert.equal(result.addressBarRequested,false);assert.equal(result.disposition,'created');
  assert.deepEqual(names(ctx),['profiles','windows.get','tabs.create']);
  assert.deepEqual(ctx.calls[2].args,[{windowId:4,active:true}]);
  assert.equal(Object.hasOwn(ctx.calls[2].args[0],'url'),false);
  assert.deepEqual(result.target,{tabId:7,windowId:4});
});
test('现有共享browser SDK接受省略url的JSON，实际桥接参数仍不含url',async()=>{
  const sdk=readFileSync(new URL('crates/omnitrigger-actions/runtime/browser-api.js',root),'utf8');
  const createBrowserApi=Function(sdk+'\nreturn createBrowserApi;')();
  const ctx=fixture();const bridge=ctx.browser;const wire=[];
  ctx.browser=createBrowserApi((method,args,options,profiles)=>{
    wire.push(JSON.parse(JSON.stringify({method:method??'profiles',args:args??[],options})));
    return profiles?bridge.profiles(options):bridge.call(method,args,options);
  },fn=>async(...args)=>fn(...args));
  assert.equal((await runWorkflow(ctx,'create-default-tab')).status,'completed');
  assert.equal(wire[2].method,'tabs.create');assert.deepEqual(wire[2].args,[{windowId:4,active:true}]);
  assert.equal(Object.hasOwn(wire[2].args[0],'url'),false);
});
test('新操作不接收URL、browser或tabId；无profile及非法参数先于任何API拒绝',async()=>{
  for(const input of [{windowId:4},{...base,url:'chrome://newtab/'},{...base,url:undefined},{...base,browser:'chrome'},
    {...base,tabId:0},{...base,windowId:-1},{...base,windowId:Number.MAX_SAFE_INTEGER+1},
    {...base,activate:false,focusWindow:true},{...base,timeoutMs:0}]){
    const ctx=fixture(input,[]);await reject(ctx,'invalid_input');assert.deepEqual(ctx.calls,[]);
  }
});
test('固定profile不可用或回执重复时停止，不选择其它连接',async()=>{
  const unavailable=fixture(base,[],[{browser_instance_id:'other'}]);await reject(unavailable,'profile_unavailable');
  const repeated=fixture(base,[],[{browser_instance_id:'synthetic-profile'},{browser_instance_id:'synthetic-profile'}]);
  await reject(repeated,'invalid_response');assert.deepEqual(names(unavailable),['profiles']);
});
test('先核验明确普通窗口，错误身份不创建',async()=>{
  for(const value of [{id:9,type:'normal'},{id:4,type:'popup'},null]){
    const ctx=fixture(base,[value]);await reject(ctx,'target_changed');assert.deepEqual(names(ctx),['profiles','windows.get']);
  }
});
test('创建回执缺身份或窗口不符时保留partial，不再创建',async()=>{
  for(const value of [{},{id:7,windowId:9}]){
    const ctx=fixture(base,[window,value]);const result=await runWorkflow(ctx,'create-default-tab');
    assert.equal(result.status,'partial');assert.equal(result.steps.length,1);
    assert.equal(names(ctx).filter(value=>value==='tabs.create').length,1);
  }
});
test('创建失败unknown或not_started准确保留，不自动重试',async()=>{
  for(const [effectState,status] of [['unknown','unknown'],['not_started','failed']]){
    const ctx=fixture(base,[window,fault('timeout',effectState)]);const result=await runWorkflow(ctx,'create-default-tab');
    assert.equal(result.status,status);assert.equal(result.error.effect_state,effectState);assert.equal(result.steps.length,0);
    assert.deepEqual(names(ctx),['profiles','windows.get','tabs.create']);
  }
});
test('显式focusWindow单独请求浏览器窗口聚焦，失败保留创建效果',async()=>{
  const ctx=fixture({...base,focusWindow:true},[window,created,fault('target_changed','not_started')]);
  const result=await runWorkflow(ctx,'create-default-tab');assert.equal(result.status,'partial');
  assert.equal(result.error.effect_state,'partial');assert.equal(result.addressBarRequested,false);
  assert.deepEqual(names(ctx),['profiles','windows.get','tabs.create','windows.update']);
});
test('首次await前快照，不被后续修改切换profile或目标',async()=>{
  const input={...base};const ctx=fixture(input,undefined,undefined,()=>{input.windowId=99;input.profile='other';input.focusWindow=true;});
  const result=await runWorkflow(ctx,'create-default-tab');assert.deepEqual(result.target,{tabId:7,windowId:4});
  assert.deepEqual(ctx.calls[2].args,[{windowId:4,active:true}]);assert.equal(ctx.calls[2].options.profile,'synthetic-profile');
  assert.equal(names(ctx).includes('windows.update'),false);
});
test('后台标签不聚焦；完全不访问桌面、前台、剪贴板或地址栏能力',async()=>{
  const ctx=fixture({...base,activate:false});
  for(const key of ['desktop','clipboard','process','open'])Object.defineProperty(ctx,key,{get(){throw new Error('禁止访问 '+key);}});
  const result=await runWorkflow(ctx,'create-default-tab');assert.equal(result.addressBarRequested,false);
  assert.deepEqual(ctx.calls[2].args,[{windowId:4,active:false}]);assert.deepEqual(names(ctx),['profiles','windows.get','tabs.create']);
});
test('浏览器默认页允许由设置或扩展决定，不读取/推断回执URL',async()=>{
  const value={...created};Object.defineProperty(value,'url',{get(){throw new Error('不读取URL');}});
  const ctx=fixture(base,[window,value]);assert.equal((await runWorkflow(ctx,'create-default-tab')).status,'completed');
});
test('原HTTP动作仍拒绝内部URL，普通URL打开行为不受默认新标签动作影响',async()=>{
  for(const url of ['chrome://newtab/','edge://newtab/','javascript:alert(1)']){
    const ctx=fixture({...base,url},[]);
    await assert.rejects(runWorkflow(ctx,'open-url'),error=>error.code==='invalid_input');assert.deepEqual(ctx.calls,[]);
  }
  const ctx=fixture({...base,url:'https://EXAMPLE.invalid/path'},[window,[],created]);
  const result=await runWorkflow(ctx,'open-url');
  assert.equal(result.status,'completed');assert.equal(result.operation,'open-url');assert.equal(result.disposition,'created');
  assert.equal(Object.hasOwn(result,'addressBarRequested'),false);
  assert.deepEqual(ctx.calls[3].args,[{windowId:4,url:'https://example.invalid/path',active:true}]);
  assert.deepEqual(names(ctx),['profiles','windows.get','tabs.query','tabs.create']);
});
