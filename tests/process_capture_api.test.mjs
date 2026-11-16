// 评估实际轻量SDK，__hostCall仅记录并返回合成JSON；没有启动任何目标进程。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
import {RecordingCaptureBackend,runRecordedCapture,normalizeCaptureWire} from '../tools/script-sdk-check/process-capture-contract.mjs';
const source=path=>readFileSync(new URL(path,import.meta.url),'utf8');
const shared=source('../crates/omnitrigger-actions/runtime/desktop-api.js')
  +source('../crates/omnitrigger-actions/runtime/browser-api.js')
  +source('../crates/omnitrigger-actions/runtime/clipboard-history-api.js')
  +source('../crates/omnitrigger-actions/runtime/process-capture-api.js');
const sdk=source('../crates/omnitrigger-actions/runtime/actions-api.js')+'\n'+source('../crates/omnitrigger-actions/runtime/state-api.js')+'\n'+source('../tools/js-engine-probe/src/sdk.js');
const executable='C:\\SyntheticOnly\\fixture.exe';
const secret='synthetic_capture_private_output';
const plain=value=>JSON.parse(JSON.stringify(value));
const fault=(code,effect='unknown')=>error=>error.code===code&&error.effectState===effect&&error.method==='process.capture'
  &&!String(error.message).includes(secret)&&error.stdout===undefined&&error.stderr===undefined&&error.requiredPermissions===undefined;
function api(reply) {
  const requests=[];
  const context={ctx:{},__hostCall:serialized=>{
    const request=JSON.parse(serialized);requests.push(request);
    const value=typeof reply==='function'?reply(request):reply;
    return JSON.stringify(value);
  }};
  runInNewContext(shared+sdk,context);
  return {process:context.ctx.process,requests};
}
const ok=value=>({ok:true,value});
const problem=(code,effect='unknown')=>({ok:false,error:{code,message:secret,effect_state:effect,method:secret,required_permissions:[secret]}});

test('实际同步capture映射已确认参数，保留既有run默认1000ms与原回执形状',()=>{
  const facade=api(request=>request.kind==='capture'?ok({code:0,stdout:'',stderr:''}):ok({pid:7,exitCode:3,success:false}));
  assert.equal(typeof facade.process.capture,'function');
  assert.deepEqual(plain(facade.process.capture(executable)),{code:0,stdout:'',stderr:''});
  assert.deepEqual(facade.requests[0],{kind:'capture',program:executable,args:[],timeout_ms:10000,allow_non_zero:false});
  facade.process.capture(executable,['a b','& echo %PATH%'],{timeoutMs:60000,allowNonZero:true,cwd:'D:\\SyntheticOnly'});
  assert.deepEqual(facade.requests[1],{kind:'capture',program:executable,args:['a b','& echo %PATH%'],timeout_ms:60000,allow_non_zero:true,cwd:'D:\\SyntheticOnly'});
  assert.deepEqual(plain(facade.process.run(executable)),{pid:7,exitCode:3,success:false});
  assert.equal(facade.requests[2].kind,'run');assert.equal(facade.requests[2].timeout_ms,1000);
});

test('实际SDK和Fake字节后端组合：分块UTF8、合法BOM/替换字符/NUL及共享容量',()=>{
  const backend=new RecordingCaptureBackend([
    {type:'stdout',bytes:Buffer.from([0xf0,0x9f]),eof:false},
    {type:'stderr',bytes:Buffer.from('\ufeff\ufffd\0'),eof:true},
    {type:'stdout',bytes:Buffer.from([0x98,0x80]),eof:true},
    {type:'poll',now:0n,code:0},
  ]);
  const facade=api(request=>ok(runRecordedCapture(normalizeCaptureWire(request),backend)));
  assert.deepEqual(plain(facade.process.capture(executable)),{code:0,stdout:'😀',stderr:'\ufeff\ufffd\0'});
  assert.equal(facade.requests.length,1);assert.equal(backend.records.at(-1).scope,'private-job');
});

test('实际SDK对输入在宿主前拒绝，未知字段/坏Unicode/容量和超时类型不转义重试',()=>{
  const bad=[['fixture'],['C:\\x.cmd'],[executable,Array(129).fill('x')],[executable,['\0']],
    [executable,['\ud800']],[executable,Array(1)],[executable,[],{timeoutMs:0}],
    [executable,[],{timeoutMs:60001}],[executable,[],{timeoutMs:'10'}],[executable,[],{timeout_ms:10}],
    [executable,[],{allowNonZero:1}],[executable,[],{cwd:null}],[executable,[],{cwd:'..'}],
    [executable,['x'.repeat(32760)]]];
  for(const args of bad) {
    const facade=api(ok({code:0,stdout:'',stderr:''}));
    assert.throws(()=>facade.process.capture(...args),fault('invalid_input','not_started'));
    assert.equal(facade.requests.length,0);
  }
});

test('实际SDK的输出容量按UTF8总字节核验，不能只检查字符串length',()=>{
  for(const [stdout,stderr] of [['x'.repeat(65536),''],['😀'.repeat(16384),''],['a'.repeat(32768),'b'.repeat(32768)]]) {
    const facade=api(ok({code:0,stdout,stderr}));
    assert.deepEqual(plain(facade.process.capture(executable)),{code:0,stdout,stderr});
  }
  for(const [stdout,stderr] of [['😀'.repeat(16385),''],['x'.repeat(65536),'x']]) {
    assert.throws(()=>api(ok({code:0,stdout,stderr})).process.capture(executable),fault('invalid_response'));
  }
});

test('实际SDK拒绝坏结果字段/i32/孤立代理项，允许非零必须显式声明',()=>{
  for(const value of [{code:null,stdout:'',stderr:''},{code:4294967295,stdout:'',stderr:''},
    {code:0,stdout:'',stderr:'',extra:secret},{code:0,stdout:'\ud800',stderr:''},{code:0,stdout:1,stderr:''}]) {
    assert.throws(()=>api(ok(value)).process.capture(executable),fault('invalid_response'));
  }
  assert.throws(()=>api(ok({code:7,stdout:secret,stderr:secret})).process.capture(executable),fault('process_exit_nonzero'));
  assert.deepEqual(plain(api(ok({code:-1,stdout:secret,stderr:''})).process.capture(executable,[],{allowNonZero:true})),{code:-1,stdout:secret,stderr:''});
});

test('实际SDK保留开始前/后错误阶段并脱敏，不返回部分输出',()=>{
  for(const code of ['invalid_arguments','unsupported_platform','process_setup_failed','process_start_failed']) {
    const facade=api(problem(code));
    assert.throws(()=>facade.process.capture(executable),fault(code,'not_started'));
    assert.equal(facade.requests.length,1);
  }
  for(const code of ['process_timeout','process_output_too_large','process_invalid_utf8','process_exit_nonzero','process_cleanup_failed','process_read_failed']) {
    const facade=api(problem(code));
    assert.throws(()=>facade.process.capture(executable),fault(code));
    assert.equal(facade.requests.length,1);
  }
  assert.throws(()=>api(problem(secret)).process.capture(executable),fault('process_capture_failed'));
});

test('外层取消的Fake记录不伪装成capture内部协作取消；同步结果不增加Promise',()=>{
  const backend=new RecordingCaptureBackend([],{cancelBefore:true});
  assert.throws(()=>runRecordedCapture({kind:'capture',program:executable},backend),error=>error.code==='cancelled'&&error.effectState==='not_started');
  assert.equal(backend.records.length,0);
  const value=api(ok({code:0,stdout:'',stderr:''})).process.capture(executable);
  assert.equal(typeof value.then,'undefined');
  // 真正取消发生在外层结束宿主，当前SDK字节桥接的伪造回执不能证明OS/Job已完成收尾。
});
