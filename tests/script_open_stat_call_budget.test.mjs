// 真SDK/通用工厂，Host只返回合成JSON；不执行延时、文件查询或打开。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
const root=new URL('../',import.meta.url),read=p=>readFileSync(new URL(p,root),'utf8');
const sdk=read('tools/js-engine-probe/src/sdk.js');
const factories=['desktop-api.js','process-capture-api.js','actions-api.js','state-api.js','browser-api.js','clipboard-history-api.js']
  .map(name=>read('crates/omnitrigger-actions/runtime/'+name)).join('\n');
function boot(available) {
  const frames=[],metadata={connection:{port:12345,token:'a'.repeat(64)},native_context_available:available};
  const scope=vm.createContext({ctx:{input:{},__nativeContext:metadata,__localDesktopAllowed:false},__hostCall:raw=>{
    const frame=JSON.parse(raw);frames.push(frame);
    return JSON.stringify({ok:true,value:frame.kind==='native'?(frame.method==='fs.stat'?{kind:'missing'}:{submitted:true}):null});
  }},{codeGeneration:{strings:false,wasm:false}});
  new vm.Script(factories+'\n'+sdk).runInContext(scope,{timeout:1000});
  return {frames,metadata,run:code=>new vm.Script(code).runInContext(scope,{timeout:1000})};
}
const ops=[
  ["ctx.open('https://synthetic.invalid/')",'system.open','desktop_call_limit',null],
  ["ctx.openFileInParent('C:/OwnedMissing/never.txt')",'system.open_file_in_parent','desktop_call_limit',null],
  ["ctx.fs.stat('C:/OwnedMissing/never.txt')",'fs.stat','fs_error',{kind:'missing'}],
];
for(const available of [false,true])for(const [code,method,errorCode,expected] of ops) {
  test(`${available}/${method}32次已用后明确错误、零Native，计数/id不再推进`,()=>{
    const f=boot(available);f.run('for(let i=0;i<32;i++)ctx.delay(0)');assert.equal(f.frames.length,32);
    assert.throws(()=>f.run(code),e=>e.code===errorCode&&e.method===method&&e.effectState==='not_started');
    assert.equal(f.frames.filter(x=>x.kind==='native').length,0);assert.equal(f.run('calls'),32);assert.equal(f.run('nativeId'),0);
    // 通用fs工厂保留readonly fs_error规范化；底层额度错误本身须完整。
    assert.throws(()=>f.run(`desktopCall(${JSON.stringify(method)},{target:'unused',path:'unused'})`),e=>e.code==='desktop_call_limit'&&e.method===method&&e.effectState==='not_started');
    assert.equal(f.frames.length,32);assert.equal(f.run('calls'),32);assert.equal(f.metadata.native_context_available,available);
  });
  test(`${available}/${method}第32次仍成功，第33次不再发送`,()=>{
    const f=boot(available);f.run('for(let i=0;i<31;i++)ctx.delay(0)');
    const value=f.run(code);assert.deepEqual(value===null?null:JSON.parse(JSON.stringify(value)),expected);
    assert.equal(f.run('calls'),32);assert.equal(f.frames.filter(x=>x.kind==='native').length,1);
    assert.throws(()=>f.run(code),e=>e.code===errorCode&&e.method===method&&e.effectState==='not_started');
    assert.equal(f.frames.length,32);assert.equal(f.run('calls'),32);assert.equal(f.run('nativeId'),1);
  });
}
test('预算修复保留32总量与capture原预提交守卫，不涉及其它API',()=>{
  assert.ok(sdk.includes('if (++calls > 32)'));
  assert.ok(sdk.includes("if ((method === 'process.capture' || opening || statPath) && calls >= 32)"));
});
