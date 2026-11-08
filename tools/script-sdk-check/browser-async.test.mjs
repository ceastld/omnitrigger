// 真实canonical工厂/Node浏览器函数片段与内存适配器；不启动应用、引擎、broker或网络。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import vm from 'node:vm';
import test from 'node:test';
import ts from 'typescript';

const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const read=name=>readFileSync(resolve(repo,'crates/omnitrigger-actions/runtime',name),'utf8');
const factory=vm.runInThisContext(`(()=>{${read('browser-api.js')};return createBrowserApi;})()`);
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const fault=(code,method,effectState='not_started')=>Object.assign(new Error('固定适配器错误'),{code,method,effectState});

test('Script同步call/profiles/chrome仍直接返回，新增API只使用独立异步回调',async()=>{
  const sync=[],async=[];
  const api=factory((method,args,options,profiles)=>{sync.push({method,args,options,profiles});return profiles?[{name:'sync'}]:7;},undefined,
    (method,args,options,profiles)=>{async.push({method,args,options,profiles});return Promise.resolve(profiles?[{name:'async'}]:8);});
  assert.equal(api.call('tabs.get',[7]),7);assert.equal(api.chrome.tabs.get(7),7);
  assert.deepEqual(api.profiles(),[{name:'sync'}]);
  const call=api.callAsync('tabs.get',[8],{profile:'work'}),profiles=api.profilesAsync({timeoutMs:1000});
  assert(call instanceof Promise);assert(profiles instanceof Promise);
  assert.equal(await call,8);assert.deepEqual(await profiles,[{name:'async'}]);
  assert.equal(sync.length,3);assert.equal(async.length,2);assert.equal(async[0].profiles,false);assert.equal(async[1].profiles,true);
  assert.equal(async[1].method,undefined);assert.equal(async[1].args,undefined);
  assert(Object.isFrozen(api));assert.deepEqual(Object.keys(api).sort(),['call','callAsync','chrome','profiles','profilesAsync'].sort());
});

test('缺少异步回调时返回Promise并在同步/异步IO之前结构化拒绝',async()=>{
  let syncIO=0;const api=factory(()=>{syncIO++;throw Error('禁止同步回退');});
  for(const [method,invoke] of [['tabs.get',()=>api.callAsync('tabs.get',[1])],['browser.profiles',()=>api.profilesAsync()]]) {
    const result=invoke();assert(result instanceof Promise);
    await assert.rejects(result,error=>error.code==='unsupported_runtime_feature'&&error.effectState==='not_started'&&error.method===method);
  }
  assert.equal(syncIO,0);
});

test('同步校验仍同步抛错；异步同校验通过Promise拒绝且不调用适配器',async()=>{
  let io=0;const api=factory(()=>{io++;},undefined,()=>{io++;return Promise.resolve();});
  const cases=[['chrome.tabs.get',[],{},'invalid_request'],['tabs.get',{}, {},'invalid_request'],
    ['tabs.get',['\ud800'],{},'invalid_request'],['tabs.get',[],null,'invalid_input'],
    ['tabs.get',[],{profile:''},'invalid_input'],['tabs.get',[],{profile:'x'.repeat(129)},'invalid_input'],
    ['tabs.get',[],{timeoutMs:0},'invalid_input'],['tabs.get',[],{unknownOption:true},'invalid_input']];
  for(const [method,args,options,code] of cases) {
    assert.throws(()=>api.call(method,args,options),error=>error.code===code&&error.effectState==='not_started');
    const result=api.callAsync(method,args,options);assert(result instanceof Promise);
    await assert.rejects(result,error=>error.code===code&&error.effectState==='not_started');
  }
  await assert.rejects(api.profilesAsync({timeoutMs:60001}),error=>error.code==='invalid_input'&&error.method==='browser.profiles'&&error.effectState==='not_started');
  assert.equal(io,0);
});

test('异步fanout在任一结果完成前已独立进入回调，参数快照不受调用方修改',async()=>{
  const seen=[],gates=[];const api=factory(()=>{throw Error('禁止同步回退');},undefined,(method,args,options,profiles)=>{
    const gate=deferred();seen.push({method,args,options,profiles});gates.push(gate);return gate.promise;
  });
  const args=[{ids:[1]}],options={profile:'work',timeoutMs:1000};
  const first=api.callAsync('tabs.get',args,options),second=api.callAsync('tabs.query',[{}]);
  assert.equal(seen.length,2);args[0].ids.push(2);options.profile='changed';
  assert.deepEqual(seen[0].args,[{ids:[1]}]);assert.equal(seen[0].options.profile,'work');
  gates[1].resolve(null);gates[0].resolve(undefined);
  assert.deepEqual(await Promise.all([first,second]),[undefined,null]);
});

test('已配置回调的错误code/method/effectState/权限原样保留且不重试',async()=>{
  for(const cause of [fault('desktop_call_limit','tabs.get'),fault('cancelled','tabs.get'),
    fault('timeout','tabs.get','unknown'),Object.assign(fault('permission_denied','tabs.get','unknown'),{requiredPermissions:['tabs']})]) {
    let dispatches=0;const api=factory(()=>{throw Error('禁止同步回退');},undefined,()=>{dispatches++;return Promise.reject(cause);});
    await assert.rejects(api.callAsync('tabs.get',[1]),error=>error===cause);
    assert.equal(dispatches,1);
  }
});

test('回调同步拒绝也成为原错误Promise；非Promise回调不伪称异步成功',async()=>{
  const cause=fault('unsupported_runtime_feature','tabs.get');
  const absent=factory(()=>{},undefined,()=>{throw cause;});
  await assert.rejects(absent.callAsync('tabs.get'),error=>error===cause);
  let dispatches=0;const malformed=factory(()=>{throw Error('禁止同步回退');},undefined,()=>{dispatches++;return 7;});
  await assert.rejects(malformed.callAsync('tabs.get'),error=>error.code==='invalid_response'&&error.effectState==='unknown'&&error.method==='tabs.get');
  assert.equal(dispatches,1);
});

const host=read('host.mjs');
const faultSource=host.slice(host.indexOf('const fault ='),host.indexOf('\nconst stop ='));
const browserSource=host.slice(host.indexOf('function checkCancelled()'),host.indexOf('\nfunction nativeCall('));
const hostBrowser=vm.runInThisContext(`(function(client,createBrowserApi){
  const controller=new AbortController();const request={browser:{}};let browserClient;
  let browserConnection=Promise.resolve(client);const browserDeadline=Date.now()+60000;const browserPending=new Set();
  ${faultSource}\n${browserSource}\nreturn {api:browser,pending:browserPending};
})`);

test('Node两个别名和原方法共用真实browserRequest派发及结果',async()=>{
  const calls=[];const client={call:async(method,args,options)=>{calls.push({method,args,options});return args[0];},profiles:async()=>[{browser_instance_id:'work',session_id:'s'}]};
  const f=hostBrowser(client,factory);
  assert.equal(await f.api.call('tabs.get',[7]),7);assert.equal(await f.api.callAsync('tabs.get',[8]),8);
  assert.deepEqual(await f.api.profilesAsync(),await f.api.profiles());
  assert.equal(calls.length,2);assert.equal(calls[0].options.signal,calls[1].options.signal);assert.equal(f.pending.size,0);
});

test('Node别名共用原8个并发上限，不另开池或绕过busy',async()=>{
  const gates=[],ready=deferred();const client={call:()=>{const gate=deferred();gates.push(gate);if(gates.length===8)ready.resolve();return gate.promise;}};
  const f=hostBrowser(client,factory),jobs=[];
  for(let index=0;index<8;index++)jobs.push(index%2?f.api.callAsync('tabs.get',[index]):f.api.call('tabs.get',[index]));
  await ready.promise;assert.equal(f.pending.size,8);
  await assert.rejects(f.api.callAsync('tabs.get',[9]),error=>error.code==='busy'&&error.effectState==='not_started');
  assert.equal(gates.length,8);gates.forEach((gate,index)=>gate.resolve(index));
  assert.deepEqual(await Promise.all(jobs),[0,1,2,3,4,5,6,7]);assert.equal(f.pending.size,0);
});

function checkTypes(runtime) {
  const file=resolve(repo,`.tmp/browser-async-${runtime}.ts`);
  const header=runtime==='node'?`import type {ActionContext} from '../crates/omnitrigger-actions/runtime/api';declare const ctx:ActionContext;`:'declare const ctx:ScriptContext;';
  const source=header+`
const result:Promise<number>=ctx.browser.callAsync<number>('tabs.get',[1],{profile:'work',timeoutMs:1000});
const profiles:Promise<Array<{browser_instance_id:string;session_id:string;name:string}>>=ctx.browser.profilesAsync({timeoutMs:1000});
// @ts-expect-error 新方法返回Promise，不能视为同步值。
const wrong:number=ctx.browser.callAsync<number>('tabs.get',[1]);
// @ts-expect-error 不增加新的选项。
ctx.browser.callAsync('tabs.get',[],{unknownOption:true});
// @ts-expect-error profiles选项保持原timeoutMs子集。
ctx.browser.profilesAsync({profile:'work'});
`+(runtime==='script'?`
const sync:number=ctx.browser.call<number>('tabs.get',[1]);
const syncProfiles:Array<{browser_instance_id:string}>=ctx.browser.profiles();
// @ts-expect-error 原Script调用仍同步，不伪装成Promise。
const oldAsync:Promise<number>=ctx.browser.call<number>('tabs.get',[1]);
const main:ScriptAsyncMain=async context=>await context.browser.callAsync<number>('tabs.get',[1]);
const oldMain:ScriptMain=async context=>await context.browser.callAsync<number>('tabs.get',[1]);
`:`const oldAsync:Promise<number>=ctx.browser.call<number>('tabs.get',[1]);`);
  const options={strict:true,exactOptionalPropertyTypes:true,noEmit:true,skipLibCheck:false,types:[],target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,moduleResolution:ts.ModuleResolutionKind.Bundler,lib:['lib.es2022.d.ts']};
  const compiler=ts.createCompilerHost(options),original=compiler.getSourceFile.bind(compiler);
  compiler.getSourceFile=(name,...args)=>resolve(name)===file?ts.createSourceFile(name,source,options.target,true):original(name,...args);
  const roots=[file,...(runtime==='script'?[resolve(repo,'crates/omnitrigger-actions/runtime/script-types.d.ts')]:[])];
  const diagnostics=ts.getPreEmitDiagnostics(ts.createProgram(roots,options,compiler)).filter(d=>d.category===ts.DiagnosticCategory.Error);
  assert.equal(diagnostics.length,0,ts.formatDiagnosticsWithColorAndContext(diagnostics,{getCurrentDirectory:()=>repo,getCanonicalFileName:f=>f,getNewLine:()=> '\n'}));
}
for(const runtime of ['node','script'])test(`browser Async：${runtime}严格类型与旧签名保持`,()=>checkTypes(runtime));
