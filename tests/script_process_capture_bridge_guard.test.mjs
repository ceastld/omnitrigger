// 真SDK和共享capture工厂；只收发内存JSON，不执行任何进程或Native。
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
const root=new URL('../',import.meta.url);
const read=path=>readFileSync(new URL(path,root),'utf8');
const sdk=read('tools/js-engine-probe/src/sdk.js');
const processApi=read('crates/omnitrigger-actions/runtime/process-capture-api.js');
const desktop=read('crates/omnitrigger-actions/runtime/desktop-api.js');
const connection={port:12345,token:'a'.repeat(64)};
const normal={code:0,stdout:'{"scope":"host_status","state":"ready"}',stderr:''};
function fixture(context,allowed,response={ok:true,value:normal}){
  const calls=[];
  const sandbox=vm.createContext({ctx:{input:{},__nativeContext:context,__localDesktopAllowed:allowed},
    __hostCall:raw=>{const frame=JSON.parse(raw);calls.push(frame);return JSON.stringify(typeof response==='function'?response(frame):response);}},
    {codeGeneration:{strings:false,wasm:false}});
  const unrelated=`function createStateApi(){return {}};function createActionsApi(){return {}};
    function createBrowserApi(){return {}};function createClipboardHistoryApi(){return {}};`;
  new vm.Script(desktop+'\n'+processApi+'\n'+unrelated+'\n'+sdk).runInContext(sandbox,{timeout:1000});
  return {calls,run:source=>new vm.Script(source).runInContext(sandbox,{timeout:1000})};
}
const call=String.raw`ctx.process.capture('C:\\Owned\\host.exe',['status','--data-root','C:\\Owned\\data'],{timeoutMs:1500,allowNonZero:true})`;
for(const available of [true,false])test(`真实Host绑定 capture桥，availability=${available}不变`,()=>{
  const f=fixture({connection,native_context_available:available,actions_scope:true},false);
  assert.deepEqual(JSON.parse(JSON.stringify(f.run(call))),normal);
  assert.equal(f.calls.length,1);assert.equal(f.calls[0].kind,'native');assert.equal(f.calls[0].method,'process.capture');
  assert.deepEqual(f.calls[0].params,{program:'C:\\Owned\\host.exe'.replaceAll('\\\\','\\'),
    args:['status','--data-root','C:\\Owned\\data'.replaceAll('\\\\','\\')],timeout_ms:1500,allow_non_zero:true});
});
test('普通参数、Unicode和cwd只序列化，不改写文本',()=>{
  const f=fixture({connection,native_context_available:false},false);
  const identity='{"count":"9007199254740993","label":"synthetic"}';
  const args=['synthetic','--literal-json',identity,'--root','C:\\Owned\\data'];
  f.run(`ctx.process.capture('C:/Owned/host.exe',${JSON.stringify(args)},{timeoutMs:25000,allowNonZero:true,cwd:'C:/Owned'})`);
  assert.deepEqual(f.calls[0].params,{program:'C:/Owned/host.exe',args,timeout_ms:25000,allow_non_zero:true,cwd:'C:/Owned'});
  assert.equal(f.calls.length,1);
});
for(const context of [null,{error:{code:'desktop_unavailable'}}])test(`明确Standalone保留内建capture：${context?'默认错误':'无context'}`,()=>{
  const f=fixture(context,true);f.run(String.raw`ctx.process.capture('C:\\Owned\\host.exe',[])`);
  assert.equal(f.calls.length,1);assert.equal(f.calls[0].kind,'capture');assert.equal(f.calls[0].timeout_ms,10000);
});
for(const context of [null,{error:{code:'native_bridge_unavailable'}},{error:{code:'desktop_unavailable'},native_context_available:false}])
test(`应用绑定缺桥/建桥失败不本地启动：${JSON.stringify(context)}`,()=>{
  const f=fixture(context,false);
  assert.throws(()=>f.run(call),e=>e.code==='native_bridge_unavailable'&&e.effectState==='not_started'&&e.method==='process.capture');
  assert.equal(f.calls.length,0);
});
test('UI-only实际Host不提供capture时保持桥拒绝，不能退内建',()=>{
  const f=fixture({connection,native_context_available:false,actions_scope:true},false,
    {ok:false,error:{code:'desktop_unavailable',message:'SENSITIVE',method:'process.capture',effect_state:'not_started'}});
  assert.throws(()=>f.run(call),e=>e.code==='desktop_unavailable'&&e.effectState==='not_started'&&!e.message.includes('SENSITIVE'));
  assert.equal(f.calls.length,1);assert.equal(f.calls[0].kind,'native');
});
for(const [code,effect] of [['cancelled','not_started'],['process_timeout','unknown'],['native_bridge_failed',undefined]])
test(`capture ${code}/${effect??'缺effect'}只提交一次并保留边界`,()=>{
  const f=fixture({connection},false,{ok:false,error:{code,message:'PRIVATE argv/output',method:'process.capture',...(effect?{effect_state:effect}:{})}});
  assert.throws(()=>f.run(call),e=>e.code===code&&e.effectState===(effect??'unknown')&&e.method==='process.capture'&&!e.message.includes('PRIVATE'));
  assert.equal(f.calls.length,1);assert.equal(f.calls[0].kind,'native');
});
test('共享工厂核对输出三字段/非零规则，坏回执unknown不重试',()=>{
  for(const value of [{code:0,stdout:12,stderr:''},{code:0,stdout:'x',stderr:'',extra:true}]){
    const f=fixture({connection},false,{ok:true,value});
    assert.throws(()=>f.run(call),e=>e.code==='invalid_response'&&e.effectState==='unknown');assert.equal(f.calls.length,1);
  }
  const value={code:9,stdout:'nonzero',stderr:'failure'};
  const allowed=fixture({connection},false,{ok:true,value});assert.deepEqual(JSON.parse(JSON.stringify(allowed.run(call))),value);
  const refused=fixture({connection},false,{ok:true,value});
  assert.throws(()=>refused.run(String.raw`ctx.process.capture('C:\\Owned\\host.exe',[])`),e=>e.code==='process_exit_nonzero'&&e.effectState==='unknown');
  assert.equal(refused.calls.length,1);
});
test('参数预检及SDK本地capture回归门都在回调前拒绝',()=>{
  const f=fixture({connection,native_context_available:false},false);
  for(const code of ["ctx.process.capture('relative.exe',[])","ctx.process.capture('C:/Owned/host.exe',[],{timeoutMs:60001})"])
    assert.throws(()=>f.run(code),e=>e.effectState==='not_started');
  assert.throws(()=>f.run("call('capture',{program:'unused'})"),e=>e.code==='local_desktop_forbidden'&&e.method==='process.capture'&&e.effectState==='not_started');
  assert.equal(f.calls.length,0);
});
test('SDK调用配额耗尽在capture提交前返回not_started',()=>{
  const f=fixture({connection},false);
  f.run("for(let i=0;i<32;i++)call('delay',{milliseconds:0})");
  assert.throws(()=>f.run(call),e=>e.code==='desktop_call_limit'&&e.effectState==='not_started'&&e.method==='process.capture');
  assert.equal(f.calls.length,32);
  assert.ok(f.calls.every(frame=>frame.kind==='delay'));
});
test('Rust白名单、长预算、错误身份与capture门接线存在，未提升false marker',()=>{
  const native=read('tools/js-engine-probe/src/native.rs'),api=read('tools/js-engine-probe/src/api.rs');
  assert.match(native,/pub const METHODS: \[&str; 47\]/);
  assert.match(native,/Duration::from_millis\(timeout \+ 1500\)/);
  assert.ok(native.indexOf('let wait_budget = method_wait_budget')<native.indexOf('TcpStream::connect_timeout'));
  assert.match(api,/Call::Capture \{ \.\. \} => Some\("process.capture"\)/);
  assert.ok(sdk.includes('const nativeContextAvailable = nativeContext && nativeContext.native_context_available !== false;'));
  assert.ok(sdk.includes("'ui.action_hint','process.capture'].includes(method)"));
});
