// 只执行共享SDK与内存回执；不运行作者、产品宿主、真实UI、输入或网络。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import vm from 'node:vm';
import test from 'node:test';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..'),runtime=resolve(root,'crates/omnitrigger-actions/runtime');
const read=file=>readFileSync(resolve(root,file),'utf8');
const desktop=read('crates/omnitrigger-actions/runtime/desktop-api.js'),host=read('crates/omnitrigger-actions/runtime/host.mjs');
const asyncLine=host.split('\n').find(line=>line.startsWith('const asyncApi ='));assert(asyncLine);
const factories=vm.runInNewContext(desktop+'\n'+asyncLine+'\n({createDesktopApi,asyncApi})');
const plain=value=>JSON.parse(JSON.stringify(value));
const fault=(code,effect='not_started')=>error=>error.code===code&&error.method==='ui.action_hint'&&error.effectState===effect;
function api(asynchronous,reply){
  const calls=[],ui=factories.createDesktopApi((method,params)=>{calls.push({method,params:plain(params)});return reply();}).ui;
  return {ui:asynchronous?factories.asyncApi(ui):ui,calls};
}
for(const asynchronous of [false,true]){
  const label=asynchronous?'Node异步':'Script同步';
  const rejected=(run,predicate)=>asynchronous?assert.rejects(run(),predicate):assert.throws(run,predicate);
  test(`${label}提示只发送text，回执不代表绘制或动作成功`,async()=>{
    const {ui,calls}=api(asynchronous,()=>({updated:true}));
    for(const text of ['v123','n10','pre','🙂'.repeat(8)]){
      const pending=ui.setActionHint({text});assert.equal(typeof pending?.then==='function',asynchronous);
      assert.deepEqual(plain(await pending),{updated:true});
    }
    assert.deepEqual(calls.map(call=>call.method),Array(4).fill('ui.action_hint'));
    assert.deepEqual(calls.map(call=>Object.keys(call.params)),Array.from({length:4},()=>['text']));
  });
  test(`${label}拒绝身份样式及无效Unicode，不执行text getter，零宿主调用`,async()=>{
    const {ui,calls}=api(asynchronous,()=>({updated:true}));let getters=0;
    const getter={get text(){getters++;return 'v123';}};
    const invalid=[undefined,null,[],{},'v123',{text:1},{text:''},{text:' v123'},{text:'v123 '},{text:'a\nb'},
      {text:'a\u0085b'},{text:'a\u2028b'},{text:'\ufeffv123'},{text:'\ud800'},{text:'🙂'.repeat(9)},
      {text:'v123',scope:{}},{text:'v123',bindingId:1},{text:'v123',invocationId:1},{text:'v123',color:'green'},getter];
    for(const options of invalid)await rejected(()=>ui.setActionHint(options),fault('invalid_input'));
    assert.equal(getters,0);assert.equal(calls.length,0);
  });
  test(`${label}坏回执为unknown，无UI错误保留且不重试`,async()=>{
    for(const receipt of [null,{},[],{updated:false},{updated:true,extra:1}]){
      const {ui,calls}=api(asynchronous,()=>receipt);await rejected(()=>ui.setActionHint({text:'v123'}),fault('invalid_response','unknown'));assert.equal(calls.length,1);
    }
    const {ui,calls}=api(asynchronous,()=>{throw Object.assign(Error('synthetic no UI'),{code:'ui_unavailable',method:'ui.action_hint',effectState:'not_started'});});
    await rejected(()=>ui.setActionHint({text:'v123'}),fault('ui_unavailable'));assert.equal(calls.length,1);
  });
}

const moduleNames=['desktop-api.js','browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js','state-api.js'];
const sdk=moduleNames.map(name=>readFileSync(resolve(runtime,name),'utf8')).join('\n')+'\n'+read('tools/js-engine-probe/src/sdk.js');
function script(scope,available=true){
  const calls=[],descriptor={actions_scope:scope,native_context_available:false,connection:{port:1,token:'synthetic-hint-only'}};
  const context=vm.createContext({ctx:{input:{actions_scope:true},invocation:{invocation_id:'forged'},__nativeContext:descriptor},__hostCall:text=>{
    const request=JSON.parse(text);calls.push(request);
    if(request.method==='ui.action_hint')return JSON.stringify(available?{ok:true,value:{updated:true}}:{ok:false,error:{code:'ui_unavailable',message:'synthetic no UI',method:'ui.action_hint',effect_state:'not_started'}});
    if(request.method==='state.read')return JSON.stringify({ok:true,value:{revision:'a'.repeat(64),entry:{present:false}}});
    if(request.method==='actions.stop_other_instances')return JSON.stringify({ok:true,value:{stoppedCount:0,effectState:'not_started'}});
    throw Error('不允许转交桌面或其他宿主方法');
  }});
  vm.runInContext(sdk,context,{timeout:1000});return {context,calls,descriptor};
}
test('真实同步SDK的scope-only提示不启用desktop，State和Scope仍可调用',()=>{
  const {context,calls,descriptor}=script(true);
  assert.deepEqual(plain(vm.runInContext("ctx.ui.setActionHint({text:'v123'})",context)),{updated:true});
  assert.equal(vm.runInContext("ctx.state.read('session','hint').entry.present",context),false);
  assert.equal(vm.runInContext('ctx.actions.stopOtherInstances().stoppedCount',context),0);
  assert.throws(()=>vm.runInContext('ctx.desktop.target()',context),error=>error.code==='desktop_unavailable'&&error.effectState==='not_started');
  assert.deepEqual(calls.map(request=>request.method),['ui.action_hint','state.read','actions.stop_other_instances']);
  assert.deepEqual(calls[0].params,{text:'v123'});assert.equal(descriptor.native_context_available,false);
});
test('无真实作用域或无UI明确失败，不把updated:true当作可渲染',()=>{
  const expired=script(false);assert.throws(()=>vm.runInContext("ctx.ui.setActionHint({text:'v123'})",expired.context),fault('ui_hint_scope_expired'));assert.equal(expired.calls.length,0);
  const absent=script(true,false);assert.throws(()=>vm.runInContext("ctx.ui.setActionHint({text:'v123'})",absent.context),fault('ui_unavailable'));assert.equal(absent.calls.length,1);
});
test('产品Node守卫保留scope-only与desktop边界，自述Windows16非Windows5',()=>{
  assert(host.includes("if (actionHint && request.actions_scope !== true)"));
  assert(host.includes('!actionControl && !actionHint && !capture && !opening && !statPath && request.native_context_available === false'));
  assert(host.includes("'state.read','state.compare_and_swap','actions.stop_other_instances'"));
  assert(!host.includes("'ui.action_hint.scoped'"));
  const source=read('tools/js-engine-probe/src/runtime_description.rs');
  const base=[...source.match(/let mut capabilities = vec!\[([\s\S]*?)\];/)[1].matchAll(/"([^"]+)"/g)].map(x=>x[1]);
  const extra=[...source.match(/if cfg!\(windows\) \{([\s\S]*?)\n    \}/)[1].matchAll(/capabilities\.push\("([^"]+)"\)/g)].map(x=>x[1]);
  assert.equal(base.length,5);assert.equal(new Set([...base,...extra]).size,16);assert(extra.includes('ui.action_hint.v1'));
  for(const capability of ['raishot.capture_result.v1','raishot.release_result.v1'])assert(extra.includes(capability));
  assert(base.includes('state.read_cas.v1')&&base.includes('actions.stop_other_instances.v1'));
  assert.match(read('tools/js-engine-probe/tests/runtime_description.rs'),/if cfg!\(windows\) \{ 16 \} else \{ 5 \}/);
});

for(const asynchronous of [false,true]) {
  test(`角标 placement ${asynchronous?'Node':'Script'} 默认wire保持、显式位置只调用一次`,async()=>{
    const {ui,calls}=api(asynchronous,()=>({updated:true}));
    for(const options of [{text:'n10'},{text:'n10',placement:'inline'},{text:'n10',placement:'topRight'}]){
      assert.deepEqual(plain(await ui.setActionHint(options)),{updated:true});
      assert.deepEqual(calls.at(-1),{method:'ui.action_hint',params:options});
    }
    assert.equal(calls.length,3);
  });
  test(`角标 placement ${asynchronous?'Node':'Script'} 坏位置与getter零Host调用`,async()=>{
    const {ui,calls}=api(asynchronous,()=>({updated:true}));let getters=0;
    for(const options of [{text:'n10',placement:null},{text:'n10',placement:undefined},{text:'n10',placement:'bottomRight'},{text:'n10',placement:1},{text:'n10',get placement(){getters++;return 'topRight';}}]){
      if(asynchronous)await assert.rejects(ui.setActionHint(options),fault('invalid_input'));
      else assert.throws(()=>ui.setActionHint(options),fault('invalid_input'));
    }
    assert.equal(getters,0);assert.equal(calls.length,0);
  });
  test(`角标 placement ${asynchronous?'Node':'Script'} 坏回执unknown且不重试`,async()=>{
    for(const value of [null,{updated:false},{updated:true,placement:'topRight'}]){
      const {ui,calls}=api(asynchronous,()=>value);
      if(asynchronous)await assert.rejects(ui.setActionHint({text:'n10',placement:'topRight'}),fault('invalid_response','unknown'));
      else assert.throws(()=>ui.setActionHint({text:'n10',placement:'topRight'}),fault('invalid_response','unknown'));
      assert.equal(calls.length,1);
    }
  });
}
