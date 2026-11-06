import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import nodeMain from '../examples/actions/browser-popout/pop-out-tab.mjs';
import {main as scriptMain} from '../examples/actions/browser-popout/pop-out-tab-script.mjs';

const apiSource=await readFile(new URL('../crates/omnitrigger-actions/runtime/browser-api.js',import.meta.url),'utf8');
const createApi=new Function(`${apiSource};return createBrowserApi;`)();
const base={profile:'owned',windowId:4,tabId:7};
const source={id:4,type:'normal',incognito:false},tab={id:7,windowId:4,incognito:false};
const popup={id:9,type:'popup',incognito:false},moved={id:7,windowId:9,incognito:false};
const fault=(effectState)=>Object.assign(new Error('合成私有错误正文'),{code:'disconnected',effectState});
function fixture({sync=true,input=base,replies=[source,tab,popup,moved],mutateInput=false}={}){
  const calls=[],ctx={input:structuredClone(input),browser:null};
  const dispatch=(method,args,options,profiles)=>{
    calls.push(structuredClone({method:profiles?'profiles':method,args:args??null,options}));
    if(profiles){if(mutateInput)Object.assign(ctx.input,{profile:'other',windowId:40,tabId:70,focusWindow:true});return[{browser_instance_id:'owned'}];}
    assert.ok(replies.length,`非预期额外调用 ${method}`);const reply=replies.shift();
    if(reply instanceof Error)throw reply;return reply;
  };
  ctx.browser=sync?createApi(dispatch):createApi(dispatch,fn=>async(...args)=>fn(...args));
  return{ctx,calls};
}
async function observe(main,options,sync){
  const f=fixture({...options,sync});
  try{const value=main(f.ctx);if(sync)assert.ok(!value||typeof value.then!=='function');
    return{result:sync?value:await value,calls:f.calls};
  }catch(e){return{error:{code:e.code,method:e.method,effectState:e.effectState},calls:f.calls};}
}
async function same(options){
  const node=await observe(nodeMain,structuredClone(options),false);
  const script=await observe(scriptMain,structuredClone(options),true);
  assert.deepEqual(script,node);return script;
}

test('同步弹出直接返回JSON，按原tabId创建并复查且不克隆URL',async()=>{
  const result=await same({mutateInput:true});
  assert.deepEqual(result.calls.map(c=>c.method),['profiles','windows.get','tabs.get','windows.create','tabs.get']);
  assert.deepEqual(result.calls[3],{method:'windows.create',args:[{tabId:7,type:'popup',focused:false,incognito:false}],options:{profile:'owned',timeoutMs:10000}});
  assert.deepEqual(result.result,{status:'completed',operation:'pop-out-tab',profile:'owned',source:{windowId:4,tabId:7},focusRequested:false,steps:[{method:'windows.create',tabId:7,windowId:9}],createdWindowId:9,disposition:'moved',target:{tabId:7,windowId:9}});
});

test('同步入口保持隐私模式与显式聚焦意图',async()=>{
  const result=await same({input:{...base,focusWindow:true},replies:[{...source,incognito:true},{...tab,incognito:true},{...popup,incognito:true},{...moved,incognito:true}]});
  assert.equal(result.result.status,'completed');assert.equal(result.calls[3].args[0].incognito,true);assert.equal(result.calls[3].args[0].focused,true);
});

test('同步前置身份和int32错误没有创建副作用',async()=>{
  const invalid=await same({input:{...base,tabId:2147483648}});assert.equal(invalid.error.code,'invalid_input');assert.deepEqual(invalid.calls,[]);
  const changed=await same({replies:[source,{...tab,incognito:true}]});assert.equal(changed.error.code,'target_changed');assert.equal(changed.calls.length,3);
});

test('同步创建unknown不重试，确认创建后的复查失败保留partial与窗口ID',async()=>{
  for(const sync of [false,true]){
    const main=sync?scriptMain:nodeMain;
    const unknown=await observe(main,{replies:[source,tab,fault('unknown')]},sync);
    assert.equal(unknown.result.status,'unknown');assert.deepEqual(unknown.result.steps,[]);assert.equal(unknown.calls.length,4);
    const partial=await observe(main,{replies:[source,tab,popup,fault('not_started')]},sync);
    assert.equal(partial.result.status,'partial');assert.equal(partial.result.createdWindowId,9);assert.equal(partial.result.error.effect_state,'partial');assert.equal(partial.calls.length,5);
    assert.ok(!JSON.stringify(partial.result).includes('合成私有'));
  }
});

test('同步创建缺少身份和移动后隐私不符不猜测target或回滚',async()=>{
  const missing=await same({replies:[source,tab,{type:'popup',incognito:false}]});assert.equal(missing.result.status,'partial');assert.equal(missing.result.createdWindowId,undefined);assert.equal(missing.calls.length,4);
  const changed=await same({replies:[source,tab,popup,{...moved,incognito:true}]});assert.equal(changed.result.status,'partial');assert.equal(changed.result.target,undefined);assert.equal(changed.result.createdWindowId,9);assert.equal(changed.calls.length,5);
});

test('同步profiles误接Promise在任何副作用前拒绝',()=>{
  let calls=0;assert.throws(()=>scriptMain({input:base,browser:{profiles:()=>Promise.resolve([{browser_instance_id:'owned'}]),call:()=>calls++}}),e=>e.code==='invalid_response');assert.equal(calls,0);
});

test('真实Script入口与调用模板匹配，原Node声明保持',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../examples/actions/browser-popout/action.json',import.meta.url)));
  const node=manifest.actions.find(a=>a.id==='pop-out-tab'),script=manifest.actions.find(a=>a.id==='pop-out-tab-script');
  assert.deepEqual(node,{id:'pop-out-tab',title:'将指定标签移入新弹出窗口',version:1,runtime:'node',entry:'pop-out-tab.mjs',single_instance:true});
  const project=JSON.parse(await readFile(new URL('../examples/actions/browser-popout/jsconfig.json',import.meta.url)));
  assert.equal(project.compilerOptions.strict,true);assert.equal(project.compilerOptions.checkJs,true);assert.ok(project.files.includes('../../../crates/omnitrigger-actions/runtime/script-types.d.ts'));
  assert.equal(manifest.actions.length,2);assert.equal(script.runtime,'script');assert.equal(script.version,1);assert.equal(script.single_instance,true);assert.equal(script.entry,'pop-out-tab-script.mjs');
  const call=JSON.parse(await readFile(new URL('../examples/actions/browser-popout/pop-out-tab-script-call.json',import.meta.url)));assert.equal(call.package_id,manifest.id);assert.equal(call.action_id,script.id);assert.equal(call.action_version,script.version);
});
