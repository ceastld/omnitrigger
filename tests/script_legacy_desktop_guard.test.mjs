// 实际SDK + 已有通用Desktop工厂的VM；回调仅返回合成JSON，绝不调用Native/OS。
import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
const root=new URL('../',import.meta.url);
const read=path=>readFileSync(new URL(path,root),'utf8');
const sdk=read('tools/js-engine-probe/src/sdk.js');
const desktop=read('crates/omnitrigger-actions/runtime/desktop-api.js');
const connection={port:12345,token:'a'.repeat(64)};
function boot(context,allowed,response,source=sdk){
  const calls=[];
  const ctx={input:{},__nativeContext:context,...(allowed===undefined?{}:{__localDesktopAllowed:allowed})};
  const sandbox=vm.createContext({ctx,__hostCall:raw=>{
    const frame=JSON.parse(raw);calls.push(frame);
    let value=response?.(frame);
    if(value===undefined)value=frame.kind==='native'&&frame.method==='clipboard.read'||frame.kind==='read_clipboard'?'synthetic text':
      frame.kind==='native'&&frame.method.startsWith('system.open')?{submitted:true}:null;
    return JSON.stringify(value?.syntheticError?{ok:false,error:value.syntheticError}:{ok:true,value});
  }},{codeGeneration:{strings:false,wasm:false}});
  const otherFactories=`function createProcessCaptureApi(){return {capture(){throw Error('not under test')}};}
    function createStateApi(){return {};};function createActionsApi(){return {};}
    function createBrowserApi(){return {};};function createClipboardHistoryApi(){return {};}`;
  new vm.Script(desktop+'\n'+otherFactories+'\n'+source).runInContext(sandbox,{timeout:1000});
  return {calls,metadata:context,run:code=>new vm.Script(code).runInContext(sandbox,{timeout:1000})};
}
const ops=[
  ['open',"ctx.open('https://synthetic.invalid/?q=a%26b')",'system.open',{target:'https://synthetic.invalid/?q=a%26b'}],
  ['parent',String.raw`ctx.openFileInParent('C:\\Synthetic\\a%20😀.txt')`,'system.open_file_in_parent',{target:'C:\\Synthetic\\a%20😀.txt'.replaceAll('\\\\','\\')}],
  ['read','ctx.clipboard.readText()','clipboard.read',{}],
  ['write',"ctx.clipboard.writeText('synthetic new text')",'clipboard.write',{text:'synthetic new text'}],
];
for(const [name,code,method,params] of ops)test(`应用桥${name}只调用现有Native方法`,()=>{
  const f=boot({connection,native_context_available:true},false);
  const value=f.run(code);
  assert.equal(value,name==='read'?'synthetic text':null);
  assert.equal(f.calls.length,1);assert.equal(f.calls[0].kind,'native');
  assert.equal(f.calls[0].method,method);assert.deepEqual(f.calls[0].params,params);
});
for(const [label,context] of [
  ['false marker',{connection,native_context_available:false}],
  ['bridge construction error',{error:{code:'native_bridge_unavailable',effect_state:'not_started'}}],
  ['application default desktop error',{error:{code:'desktop_unavailable',effect_state:'not_started'}}],
  ['application no context',null]
])for(const [name,code,method,params] of ops)test(`${label}/${name}不退本地`,()=>{
  const f=boot(context,false);
  // false只描述原目标上下文；两个opening方法不依赖它，交实际Host判断。
  if(label==='false marker'&&(name==='open'||name==='parent')) {
    assert.equal(f.run(code),null);
    assert.equal(f.calls.length,1);assert.equal(f.calls[0].kind,'native');
    assert.equal(f.calls[0].method,method);assert.deepEqual(f.calls[0].params,params);
    assert.equal(f.metadata.native_context_available,false);
  } else {
    assert.throws(()=>f.run(code),error=>error.effectState==='not_started');
    assert.equal(f.calls.length,0);
  }
});
for(const [label,context] of [['null',null],['default error',{error:{code:'desktop_unavailable'}}],['unrelated metadata',{foo:true}]])
test(`真正Standalone ${label}保留旧路由和返回`,()=>{
  const f=boot(context,true);
  assert.equal(f.run("ctx.open('https://synthetic.invalid/')"),null);
  assert.equal(f.run('ctx.clipboard.readText()'),'synthetic text');
  assert.equal(f.run("ctx.clipboard.writeText('new synthetic')"),null);
  assert.equal(f.run(String.raw`ctx.openFileInParent('C:\\Synthetic\\a.txt')`),null);
  assert.deepEqual(f.calls.map(c=>c.kind),['open','read_clipboard','write_clipboard','open_file_in_parent']);
  assert.throws(()=>f.run("ctx.clipboard.writeText('x',{excludeFromHistory:true})"),e=>e.code==='invalid_input');
});
test('旧生产者真实connection元数据进入桥；不凭任意对象猜测',()=>{
  const f=boot({connection},undefined);assert.equal(f.run("ctx.open('https://synthetic.invalid/')"),null);
  assert.equal(f.calls[0].kind,'native');
});
for(const [kind,method] of Object.entries({open:'system.open',open_file_in_parent:'system.open_file_in_parent',read_clipboard:'clipboard.read',write_clipboard:'clipboard.write'}))
test(`SDK旧分派回归${kind}在回调前被禁止`,()=>{
  const f=boot(null,false);
  assert.throws(()=>f.run(`call(${JSON.stringify(kind)},{target:'unused',text:'unused'})`),
    e=>e.code==='local_desktop_forbidden'&&e.method===method&&e.effectState==='not_started');
  assert.equal(f.calls.length,0);
});
test('bridge unknown只提交一次，不回退；成功回执坏型也不重试',()=>{
  const f=boot({connection},false,()=>({syntheticError:{code:'open_failed',effect_state:'unknown',method:'system.open',message:'synthetic'}}));
  assert.throws(()=>f.run("ctx.open('https://synthetic.invalid/')"),e=>e.code==='open_failed'&&e.effectState==='unknown');
  assert.equal(f.calls.length,1);assert.equal(f.calls[0].kind,'native');
  for(const value of [null,{}, {submitted:false},{submitted:true,extra:true}]){
    const f=boot({connection},false,()=>value);
    assert.throws(()=>f.run("ctx.open('https://synthetic.invalid/')"),e=>e.code==='invalid_response'&&e.effectState==='unknown');
    assert.equal(f.calls.length,1);
  }
});
test('参数与父目录路径守卫不被桥接改写',()=>{
  const f=boot({connection},false);
  for(const value of ['undefined','null','42','{}'])assert.throws(()=>f.run(`ctx.open(${value})`),e=>e.code==='invalid_arguments');
  for(const code of ["ctx.openFileInParent('relative.txt')","ctx.openFileInParent('C:/a.txt','extra')"])
    assert.throws(()=>f.run(code),e=>e.code==='invalid_input');
  assert.equal(f.calls.length,0);
});
for(const [name,code,method,params] of ops.slice(0,2)) {
  for(const [errorCode,effect] of [['open_unavailable','not_started'],['open_failed','unknown']])
  test(`false/${name}/${effect}保持Host判断，无本地回退`,()=>{
    const f=boot({connection,native_context_available:false},false,()=>({syntheticError:{code:errorCode,effect_state:effect,method,message:'synthetic'}}));
    assert.throws(()=>f.run(code),e=>e.code===errorCode&&e.effectState===effect&&e.method===method);
    assert.equal(f.calls.length,1);assert.equal(f.calls[0].kind,'native');
    assert.equal(f.calls[0].method,method);assert.deepEqual(f.calls[0].params,params);
    assert.equal(f.metadata.native_context_available,false);
  });
  test(`false/${name}缺connection在回调前拒绝`,()=>{
    const f=boot({native_context_available:false},false);
    assert.throws(()=>f.run(code),e=>e.code==='native_bridge_unavailable'&&e.effectState==='not_started'&&e.method===method);
    assert.equal(f.calls.length,0);assert.equal(f.metadata.native_context_available,false);
  });
}
test('false仍在回调前拒绝窗口和剪贴板方法',()=>{
  for(const method of ['window.geometry','clipboard.read','clipboard.write']) {
    const f=boot({connection,native_context_available:false},false);
    assert.throws(()=>f.run(`desktopCall(${JSON.stringify(method)},{})`),e=>e.effectState==='not_started'&&e.method===method);
    assert.equal(f.calls.length,0);assert.equal(f.metadata.native_context_available,false);
  }
});
test('Rust运行层在执行前绑定并在旧桌面分派前检查；生产者按实际Host生成模式',()=>{
  const api=read('tools/js-engine-probe/src/api.rs'),main=read('tools/js-engine-probe/src/main.rs');
  const host=read('crates/omnitrigger-actions/src/script_host.rs');
  assert.match(api,/check_local_desktop\(&call\)\?;\s*execute\(call\)/);
  for(const method of ['system.open','system.open_file_in_parent','clipboard.read','clipboard.write'])assert.ok(api.includes(`Some("${method}")`));
  assert.ok(main.indexOf('DesktopScope::enter(desktop_mode)')<main.indexOf('evaluate(&source'));
  assert.match(host,/"desktop_mode":if host\.is_some\(\) \{ "application_bridge" \} else \{ "standalone" \}/);
  assert.ok(host.includes('"native_context_available":host.is_some_and(|host| host.native_context_available())'));
  const native=read('tools/js-engine-probe/src/native.rs');
  assert.ok(native.includes('"system.open"')&&native.includes('"system.open_file_in_parent"'));

});
