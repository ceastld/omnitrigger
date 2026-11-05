// 只注入合成桥接，不启动程序、App、网络或剪贴板。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
const source=readFileSync(new URL('../crates/omnitrigger-actions/runtime/process-capture-api.js',import.meta.url),'utf8');
const factory=runInNewContext(source+'\ncreateProcessCaptureApi;',Object.create(null));
const program='C:\\OwnedFixture\\capture.exe';
const good={code:0,stdout:'合成 😀\0',stderr:'diagnostic'};
function setup(asyncMode,result=good) {
  const calls=[];
  const api=factory(params=>{calls.push(JSON.parse(JSON.stringify(params)));return asyncMode?Promise.resolve(result):result;},asyncMode);
  return {api,calls};
}
const plain=value=>JSON.parse(JSON.stringify(value));
async function rejects(operation,code,effectState) {
  await assert.rejects(async()=>operation(),error=>{
    assert.equal(error.code,code);assert.equal(error.effectState,effectState);assert.equal(error.method,'process.capture');
    assert(!error.message.includes('PRIVATE_PAYLOAD'));return true;
  });
}
for(const asyncMode of [false,true]) {
  const mode=asyncMode?'Node异步':'轻量同步';
  test(mode+'默认参数和严格结果共享同一契约',async()=>{
    const {api,calls}=setup(asyncMode);
    assert.deepEqual(plain(await api.capture(program)),good);
    assert.deepEqual(calls,[{program,args:[],timeout_ms:10000,allow_non_zero:false}]);
    assert(Object.isFrozen(api));assert(!Object.hasOwn(api,'run'));
  });
  test(mode+'未知选项、坏Unicode、稀疏参数在桥接前拒绝',async()=>{
    const invalid=[
      ['relative.exe'],['\\\\server\\x.exe'],['C:\\Owned\\x.cmd'],[program,[1]],
      [program,new Array(1)],[program,['\ud800']],[program,[],{cwd:null}],
      [program,[],{timeoutMs:0}],[program,[],{timeoutMs:60001}],[program,[],{timeoutMs:1.2}],
      [program,[],{allowNonZero:1}],[program,[],{shell:true}],[program,[],{timeoutMs:undefined}],
      ['C:\\Owned\\..\\capture.exe'],[program,[],{cwd:'C:\\Owned\\.'}],
    ];
    for(const args of invalid) {
      const {api,calls}=setup(asyncMode);
      await rejects(()=>api.capture(...args),'invalid_input','not_started');assert.equal(calls.length,0);
    }
  });
  test(mode+'非零码默认拒绝，显式允许时原码返回',async()=>{
    const {api}=setup(asyncMode,{code:7,stdout:'',stderr:'PRIVATE_PAYLOAD'});
    await rejects(()=>api.capture(program),'process_exit_nonzero','unknown');
    assert.equal((await api.capture(program,[],{allowNonZero:true,timeoutMs:60000,cwd:'C:\\Owned'})).code,7);
  });
  test(mode+'输出按UTF8字节合计64KiB，不按JS字符或流分别放宽',async()=>{
    const exact=setup(asyncMode,{code:0,stdout:'x'.repeat(65535),stderr:'y'});
    assert.equal((await exact.api.capture(program)).stdout.length,65535);
    for(const result of [
      {code:0,stdout:'x'.repeat(65536),stderr:'y'},
      {code:0,stdout:'中'.repeat(22000),stderr:''},
      {code:0,stdout:'\ud800',stderr:''},
      {code:0,stdout:'',stderr:'',extra:true},
      {code:1.5,stdout:'',stderr:''},
    ]) await rejects(()=>setup(asyncMode,result).api.capture(program),'invalid_response','unknown');
  });
  test(mode+'取消、外层期限、Job清理和桥接错误只返回固定结构',async()=>{
    for(const code of ['cancelled','timeout','process_cleanup_failed','process_invalid_utf8','native_bridge_unavailable','desktop_busy','unsupported_platform']) {
      const effectState=['native_bridge_unavailable','desktop_busy','unsupported_platform'].includes(code)?'not_started':'unknown';
      const api=factory(()=>{throw Object.assign(Error('PRIVATE_PAYLOAD'),{code,effectState,requiredPermissions:['invented']});},asyncMode);
      await rejects(()=>api.capture(program),code,effectState);
    }
  });
}
test('Node桥接的异步拒绝被收尾，不退回旧run或回显异常内容',async()=>{
  let calls=0;
  const api=factory(()=>{calls++;return Promise.reject(Object.assign(Error('PRIVATE_PAYLOAD'),{code:'process_timeout',effectState:'unknown'}));},true);
  await rejects(()=>api.capture(program),'process_timeout','unknown');assert.equal(calls,1);
});
test('命令行UTF16和引号膨胀上限在派发前校验',async()=>{
  for(const asyncMode of [false,true]) {
    const {api,calls}=setup(asyncMode);
    await rejects(()=>api.capture(program,['"'.repeat(17000)]),'invalid_input','not_started');
    await rejects(()=>api.capture(program,Array.from({length:129},()=>'')),'invalid_input','not_started');
    assert.equal(calls.length,0);
  }
});
