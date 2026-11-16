import assert from 'node:assert/strict';
import {test} from 'node:test';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
const root=new URL('../',import.meta.url);
const source=await readFile(new URL('crates/omnitrigger-actions/runtime/state-api.js',root),'utf8');
const create=Function(source+'\nreturn createStateApi;')();
const rev='a'.repeat(64),nextRev='b'.repeat(64);
const snapshot=(entry={present:false},revision=rev)=>({revision,entry});
const json=value=>JSON.parse(JSON.stringify(value));
const check=(code,effectState,method)=>error=>{assert.equal(error.code,code);assert.equal(error.effectState,effectState);assert.equal(error.method,method);assert(!error.message.includes('private'));return true;};
for(const asyncMode of [false,true]){
 const label=asyncMode?'Node异步':'Script同步';
 const rejects=async(fn,predicate)=>asyncMode?assert.rejects(fn(),predicate):assert.throws(fn,predicate);
 test(`${label}：missing、null、0及空串保持区别且只传两方法`,async()=>{
  for(const entry of [{present:false},{present:true,value:null},{present:true,value:0},{present:true,value:''}]){
   const calls=[];const api=create((method,params)=>{calls.push({method,params});return snapshot(entry);},asyncMode);
   assert.deepEqual(await api.read('session','键'),snapshot(entry));assert.deepEqual(calls,[{method:'state.read',params:{area:'session',key:'键'}}]);
  }
 });
 test(`${label}：CAS提交与冲突原样返回，不自动重试`,async()=>{
  for(const status of ['committed','conflict']){const calls=[];const api=create((method,params)=>{calls.push({method,params});return {status,current:snapshot({present:true,value:null},nextRev)};},asyncMode);assert.deepEqual(await api.compareAndSwap('durable','choice',rev,{present:true,value:null}),{status,current:snapshot({present:true,value:null},nextRev)});assert.deepEqual(calls,[{method:'state.compare_and_swap',params:{area:'durable',key:'choice',expectedRevision:rev,next:{present:true,value:null}}}]);}
 });
 test(`${label}：拒绝自报身份、额外参数、getter和非JSON值，零调用`,async()=>{
  let calls=0;const api=create(()=>{calls++;},asyncMode);const getter={present:true};Object.defineProperty(getter,'value',{enumerable:true,get(){assert.fail('不能调用getter');}});
  for(const args of [['session','key',rev,{present:false,value:0}],['session','key',rev,{present:true}],['session','key',rev,{present:true,value:undefined}],['session','key',rev,getter],['session','key',rev,{present:true,value:BigInt(1)}],['session','key',rev,{present:true,value:NaN}],['session','key',rev,{present:true,value:9007199254740992}],['session','key',rev,{present:false},{actionId:'other'}]])await rejects(()=>api.compareAndSwap(...args),check('invalid_input','not_started','state.compare_and_swap'));
  for(const args of [['global','key'],['session',''],['session','x'.repeat(65)],['session','键'.repeat(22)],['session','key',{actionId:'other'}]])await rejects(()=>api.read(...args),check('invalid_input','not_started','state.read'));assert.equal(calls,0);
 });
 test(`${label}：大小、深度、节点、循环与孤立代理项边界`,async()=>{
  let calls=0;const api=create(()=>{calls++;},asyncMode);let deep=null;for(let i=0;i<9;i++)deep=[deep];const cycle={};cycle.self=cycle;
  for(const value of ['x'.repeat(16384),deep,Array(2049).fill(null),cycle,'\ud800',{'\udfff':0},new Date(),[,0]])await rejects(()=>api.compareAndSwap('session','key',rev,{present:true,value}),check('invalid_input','not_started','state.compare_and_swap'));assert.equal(calls,0);
 });
 test(`${label}：坏回执读操作无提交，CAS未知效果不重试`,async()=>{
  for(const result of [null,{},snapshot({present:false,value:0}),snapshot({present:true}),{revision:'bad',entry:{present:false}},{...snapshot(),actionId:'forged'}]){let calls=0;const api=create(()=>{calls++;return result;},asyncMode);await rejects(()=>api.read('session','key'),check('invalid_response','not_started','state.read'));assert.equal(calls,1);}
  let calls=0;const api=create(()=>{calls++;return {status:'ok',current:snapshot()};},asyncMode);await rejects(()=>api.compareAndSwap('session','key',rev,{present:false}),check('invalid_response','unknown','state.compare_and_swap'));assert.equal(calls,1);
 });
 test(`${label}：结构化存储失败/取消保留，敏感异常正文不回显`,async()=>{
  for(const code of ['state_io','state_corrupt','state_storage_changed','state_unavailable','cancelled','timeout']){let calls=0;const api=create(()=>{calls++;throw Object.assign(Error('private contents'),{code,effectState:'not_started'});},asyncMode);await rejects(()=>api.compareAndSwap('durable','key',rev,{present:true,value:0}),check(code,'not_started','state.compare_and_swap'));assert.equal(calls,1);}
  const api=create(()=>{throw Error('private contents');},asyncMode);await rejects(()=>api.compareAndSwap('session','key',rev,{present:false}),check('state_failed','unknown','state.compare_and_swap'));
 });
 test(`${label}：输入和回执值复制，不创建共享模块状态`,async()=>{
  const calls=[],raw=snapshot({present:true,value:{list:[1]}});const api=create((_m,p)=>{calls.push(p);return {status:'committed',current:raw};},asyncMode);const value={list:[1]};const receipt=await api.compareAndSwap('session','key',rev,{present:true,value});value.list.push(2);raw.entry.value.list.push(3);assert.deepEqual(calls[0].next.value,{list:[1]});assert.deepEqual(receipt.current.entry.value,{list:[1]});assert(Object.isFrozen(receipt));assert(Object.isFrozen(receipt.current));
 });
}
test('两运行时协议视图可复用同一宿主；此端口不代替Rust服务并发测试',async()=>{
 let current=snapshot();const invoke=(method,params)=>{if(method==='state.read')return current;assert.equal(params.expectedRevision,current.revision);current=snapshot(params.next,nextRev);return {status:'committed',current};};
 const node=create(invoke,true),script=create(invoke,false);const old=await node.read('session','key');script.compareAndSwap('session','key',old.revision,{present:true,value:'shared'});assert.equal((await node.read('session','key')).entry.value,'shared');
});
test('真实同步SDK构造接线允许无桌面上下文的服务状态；未提供服务明确不可用',async()=>{
 const sdk=await readFile(new URL('tools/js-engine-probe/src/sdk.js',root),'utf8');
 for(const scope of [false,true]){
  const calls=[];const context={ctx:{input:{},__nativeContext:{actions_scope:scope,native_context_available:false,connection:{}}},__hostCall:raw=>{const r=JSON.parse(raw);calls.push(r);return JSON.stringify({ok:true,value:snapshot()});},createDesktopApi:()=>({fs:{},desktop:{},clipboard:{},ui:{}}),createBrowserApi:()=>({}),createClipboardHistoryApi:()=>({}),createProcessCaptureApi:()=>({capture(){}}),createActionsApi:()=>({})};
  vm.createContext(context);vm.runInContext(source+'\n'+sdk,context);
  if(scope){assert.deepEqual(json(vm.runInContext("ctx.state.read('session','key')",context)),snapshot());assert.equal(calls.length,1);assert.equal(calls[0].kind,'native');assert.equal(calls[0].method,'state.read');for(let i=1;i<32;i++)vm.runInContext("ctx.state.read('session','key')",context);assert.throws(()=>vm.runInContext("ctx.state.read('session','key')",context),check('state_limit','not_started','state.read'));assert.equal(calls.length,32);}
  else{assert.throws(()=>vm.runInContext("ctx.state.read('session','key')",context),check('state_unavailable','not_started','state.read'));assert.equal(calls.length,0);}
 }
});
test('Node上下文、两桥白名单、两SDK和运行时打包都有真实接点',async()=>{
 for(const [path,tokens] of [
  ['crates/omnitrigger-actions/runtime/host.mjs',['state:createStateApi','state.read','state.compare_and_swap','request.actions_scope']],
  ['crates/omnitrigger-actions/src/node.rs',['const STATE_API','{STATE_API}']],
  ['tools/js-engine-probe/src/main.rs',['const STATE_API','{STATE_API}']],
  ['crates/omnitrigger-actions/src/native.rs',['"state.read"','"state.compare_and_swap"']],
  ['tools/js-engine-probe/src/native.rs',['"state.read"','"state.compare_and_swap"']],
  ['crates/omnitrigger-actions/runtime/api.d.ts',['state: ActionStateApi','Promise<ActionStateSnapshot>','Promise<ActionStateCas>']],
  ['crates/omnitrigger-actions/runtime/script-types.d.ts',['state: ScriptActionStateApi','): ScriptActionStateSnapshot','): ScriptActionStateCas']],
 ]){const text=await readFile(new URL(path,root),'utf8');for(const token of tokens)assert(text.includes(token),`${path} missing ${token}`);}
});
test('Node未await状态操作：正式收尾片段区分read与CAS效果且不伪报成功',async()=>{
 const host=await readFile(new URL('crates/omnitrigger-actions/runtime/host.mjs',root),'utf8');
 const start=host.indexOf('  if (nativePending) {');const end=host.indexOf("\n  respond({ status: 'ok'",start);
 assert(start>0&&end>start);const pending=Function('nativePending','nativePendingMethod','fault',host.slice(start,end));
 for(const [method,effect]of [['state.read','not_started'],['state.compare_and_swap','unknown']])assert.throws(()=>pending(true,method,(code,message)=>Object.assign(Error(message),{code})),check('pending_state_call',effect,method));
});
