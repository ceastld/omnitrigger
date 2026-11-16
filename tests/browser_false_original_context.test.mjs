// 完整公开SDK与真实Node浏览器适配函数；回调/已连接client都是内存替身。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
const root=new URL('../',import.meta.url),read=p=>readFileSync(new URL(p,root),'utf8');
const sdk=read('tools/js-engine-probe/src/sdk.js');
const factories=['desktop-api.js','process-capture-api.js','actions-api.js','state-api.js','browser-api.js','clipboard-history-api.js']
  .map(name=>read('crates/omnitrigger-actions/runtime/'+name)).join('\n');
const connection={port:12345,token:'a'.repeat(64)};
const plain=value=>JSON.parse(JSON.stringify(value));
function boot(metadata={connection,native_context_available:false},reply) {
  const frames=[],scope=vm.createContext({ctx:{input:{},__nativeContext:metadata,__localDesktopAllowed:false},__hostCall:raw=>{
    const frame=JSON.parse(raw);frames.push(frame);
    return JSON.stringify(reply?.(frame)??{ok:true,value:frame.method==='browser.profiles'?[]:{has_value:false}});
  }},{codeGeneration:{strings:false,wasm:false}});
  new vm.Script(factories+'\n'+sdk).runInContext(scope,{timeout:1000});
  return {frames,metadata,run:code=>new vm.Script(code).runInContext(scope,{timeout:1000})};
}
const operations=[
  ['call',"ctx.browser.call('tabs.get',[7],{profile:'synthetic-work',timeoutMs:37})",'browser.call',{method:'tabs.get',args:[7],options:{profile:'synthetic-work',timeoutMs:37}},'tabs.get'],
  ['profiles','ctx.browser.profiles({timeoutMs:37})','browser.profiles',{options:{timeoutMs:37}},'browser.profiles'],
];
for(const [label,code,method,params,publicMethod] of operations) {
  test(`Script false/${label}只交实际Host，参数/false/原同步返回合同保持`,()=>{
    const result=label==='call'?{id:7,windowId:3}:[{browser_instance_id:'synthetic-work',session_id:'synthetic-session'}];
    const f=boot(undefined,()=>({ok:true,value:label==='call'?{has_value:true,value:result}:result}));
    assert.deepEqual(plain(f.run(code)),result);assert.equal(f.frames.length,1);
    assert.equal(f.frames[0].kind,'native');assert.equal(f.frames[0].method,method);assert.deepEqual(f.frames[0].params,params);
    assert.equal(f.metadata.native_context_available,false);
  });
  for(const [errorCode,effect] of [['browser_unavailable','not_started'],['desktop_unavailable','not_started'],['disconnected','unknown']])
  test(`Script false/${label}/${effect}保留Host错误与原方法，不回退或重试`,()=>{
    const f=boot(undefined,()=>({ok:false,error:{code:errorCode,message:'synthetic error',effect_state:effect,method}}));
    assert.throws(()=>f.run(code),e=>e.code===errorCode&&e.effectState===effect&&e.method===publicMethod);
    assert.equal(f.frames.length,1);assert.equal(f.frames[0].kind,'native');assert.equal(f.metadata.native_context_available,false);
  });
  test(`Script ${label}缺桥/构造失败在任何回调前拒绝`,()=>{
    for(const metadata of [null,{native_context_available:false},{connection:null,native_context_available:false},
      {error:{code:'native_bridge_unavailable',effect_state:'not_started'},native_context_available:false},
      {connection,error:{code:'native_bridge_unavailable',effect_state:'not_started'},native_context_available:false}]) {
      const f=boot(metadata);
      assert.throws(()=>f.run(code),e=>e.effectState==='not_started'&&e.method===publicMethod&&['browser_unavailable','native_bridge_unavailable'].includes(e.code));
      assert.equal(f.frames.length,0);if(metadata)assert.equal(metadata.native_context_available,false);
    }
  });
}
test('Script false的undefined/null和坏回执沿既有浏览器合同，不重试',()=>{
  for(const [value,expected] of [[{has_value:false},undefined],[{has_value:true,value:null},null]]) {
    const f=boot(undefined,()=>({ok:true,value}));assert.equal(f.run("ctx.browser.call('tabs.get',[7])"),expected);assert.equal(f.frames.length,1);
  }
  const bad=boot(undefined,()=>({ok:true,value:{}}));
  assert.throws(()=>bad.run("ctx.browser.call('tabs.get',[7])"),e=>e.code==='invalid_response'&&e.method==='tabs.get'&&e.effectState==='unknown');
  assert.equal(bad.frames.length,1);
});
test('parity仅两方法：Window/Clipboard及其它browser Native方法仍守false',()=>{
  for(const method of ['window.geometry','clipboard.read','clipboard.write','browser.other']) {
    const f=boot();assert.throws(()=>f.run(`desktopCall(${JSON.stringify(method)},{})`),e=>e.effectState==='not_started'&&e.method===method);
    assert.equal(f.frames.length,0);assert.equal(f.metadata.native_context_available,false);
  }
});
test('parity不增加browser选项、额度或私自选择profile',()=>{
  const f=boot();assert.throws(()=>f.run("ctx.browser.call('tabs.get',[7],{expectedSessionId:'synthetic'})"),e=>e.code==='invalid_input');
  assert.equal(f.frames.length,0);assert.ok(sdk.includes('if (++calls > 32)'));assert.equal(f.run('calls'),0);
});

const host=read('crates/omnitrigger-actions/runtime/host.mjs');
const functions=host.slice(host.indexOf('function browserOptions('),host.indexOf('const browser = createBrowserApi('));
const browserFactory=read('crates/omnitrigger-actions/runtime/browser-api.js');
function nodeBrowser({missing=false,error}={}) {
  const calls=[],client={call:async(method,args,options)=>{calls.push({method,args,options});if(error)throw error;return {id:7};},
    profiles:async options=>{calls.push({method:'profiles',options});if(error)throw error;return [];}},controller=new AbortController();
  const scope=vm.createContext({request:{native_context_available:false,...(missing?{}:{browser:{sdk_path:'synthetic-no-import',config_path:'synthetic-no-read'}})},
    controller,browserDeadline:Date.now()+10000,browserPending:new Set(),browserConnection:Promise.resolve(client),browserClient:client,
    fault:(code,message)=>Object.assign(Error(message),{code}),checkCancelled:()=>{if(controller.signal.aborted)throw controller.signal.reason;}});
  const api=vm.runInContext(browserFactory+'\n'+functions+'\ncreateBrowserApi(browserRequest,fn=>async(...args)=>fn(...args));',scope);
  return {api,calls,scope};
}
for(const profiles of [false,true]) {
  test(`Node false/${profiles?'profiles':'call'}使用独立已连接浏览器桥，不需要Native上下文`,async()=>{
    const n=nodeBrowser();const result=profiles?await n.api.profiles({timeoutMs:37}):await n.api.call('tabs.get',[7],{profile:'synthetic-work',timeoutMs:37});
    assert.deepEqual(plain(result),profiles?[]:{id:7});assert.equal(n.calls.length,1);
    assert.equal(n.calls[0].method,profiles?'profiles':'tabs.get');assert.equal(n.calls[0].options.timeoutMs,37);
    assert.equal(n.scope.request.native_context_available,false);assert.equal(n.scope.request.native,undefined);
  });
  test(`Node false/${profiles?'profiles':'call'}缺专用桥not_started，Host异常不重试`,async()=>{
    const missing=nodeBrowser({missing:true});
    await assert.rejects(profiles?missing.api.profiles():missing.api.call('tabs.get',[7]),e=>e.code==='browser_unavailable'&&e.effectState==='not_started');
    assert.equal(missing.calls.length,0);
    const error=Object.assign(Error('synthetic'),{code:'disconnected',effectState:'unknown'}),n=nodeBrowser({error});
    await assert.rejects(profiles?n.api.profiles():n.api.call('tabs.get',[7]),e=>e===error);
    assert.equal(n.calls.length,1);assert.equal(n.scope.request.native_context_available,false);
  });
}
