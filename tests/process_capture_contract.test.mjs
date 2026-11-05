// 合成字节与记录后端；不创建或终止真实进程，不读取系统数据。
import assert from 'node:assert/strict';
import test from 'node:test';
import {RecordingCaptureBackend,runRecordedCapture,recordedCaptureAdapter,normalizeCaptureWire,validateCapturedValue} from '../tools/script-sdk-check/process-capture-contract.mjs';
const request={kind:'capture',program:'C:\\SyntheticOnly\\fixture.exe'};
const secret='synthetic-private-capture-body';
const out=(bytes,eof=true)=>({type:'stdout',bytes:Buffer.from(bytes),eof});
const err=(bytes,eof=true)=>({type:'stderr',bytes:Buffer.from(bytes),eof});
const poll=(code=0,now=0n)=>({type:'poll',code,now});
const fault=(code,effect='unknown')=>error=>error.code===code&&error.method==='process.capture'
  &&error.effectState===effect&&!String(error.message).includes(secret)
  &&error.stdout===undefined&&error.stderr===undefined&&error.requiredPermissions===undefined;
function run(events,options={},backendOptions={}) {
  const backend=new RecordingCaptureBackend(events,backendOptions);
  return {backend,invoke:()=>runRecordedCapture({...request,...options},backend)};
}

test('已确认wire默认值、严格字段和EXE参数；验参错误不进入start',()=>{
  assert.deepEqual(normalizeCaptureWire(request),{...request,args:[],timeout_ms:10000,allow_non_zero:false,cwd:null});
  const values=[{...request,extra:secret},{...request,program:'fixture'},{...request,program:'C:\\x.cmd'},
    {...request,program:'\\\\server\\fixture.exe'},{...request,program:'C:\\..\\fixture.exe'},
    {...request,args:Array(129).fill('x')},{...request,args:['\ud800']},{...request,args:['\0']},
    {...request,timeout_ms:0},{...request,timeout_ms:60001},{...request,timeout_ms:1.1},
    {...request,allow_non_zero:1},{...request,cwd:'relative'},{...request,args:Array(1)},
    {...request,args:['x'.repeat(32760)]}];
  for(const value of values) {
    const backend=new RecordingCaptureBackend([]);
    assert.throws(()=>runRecordedCapture(value,backend),fault('invalid_arguments','not_started'));
    assert.equal(backend.records.length,0);
  }
  const literal=normalizeCaptureWire({...request,args:['','a b','&echo %PATH%','末尾\\','a"b'],timeout_ms:60000,allow_non_zero:true,cwd:'D:\\SyntheticOnly'});
  assert.deepEqual(literal.args,['','a b','&echo %PATH%','末尾\\','a"b']);
});

test('同步与Promise记录适配器给出相同回执，不重试且成功后只确认一次Job收尾',async()=>{
  for(const asynchronous of [false,true]) {
    const backend=new RecordingCaptureBackend([out(' \r\n\0中文😀 '),err(' stderr '),poll()]);
    const value=recordedCaptureAdapter(asynchronous,backend)(request);
    assert.equal(typeof value?.then==='function',asynchronous);
    assert.deepEqual(await value,{code:0,stdout:' \r\n\0中文😀 ',stderr:' stderr '});
    assert.deepEqual(backend.records.map(record=>record.kind),['start','finish']);
    assert.equal(backend.records[1].scope,'private-job');
  }
});

test('stdout/stderr共用65536字节预算，不能按UTF16单元或每流分别算',()=>{
  for(const [stdout,stderr] of [['x'.repeat(65536),''],['😀'.repeat(16384),''],['中'.repeat(21845),'x'],['a'.repeat(32768),'b'.repeat(32768)]]) {
    const {invoke}=run([out(stdout),err(stderr),poll()]);
    assert.deepEqual(invoke(),{code:0,stdout,stderr});
  }
  for(const [stdout,stderr] of [['x'.repeat(65537),''],['😀'.repeat(16384),'x'],['a'.repeat(32769),'b'.repeat(32768)]]) {
    const {backend,invoke}=run([out(stdout),err(stderr),poll()]);
    assert.throws(invoke,fault('process_output_too_large'));
    assert.deepEqual(backend.records.map(item=>item.kind),['start','finish']);
  }
});

test('先分别合并两条流再严格解码，保留合法替换字符、BOM、空白与NUL',()=>{
  const emoji=Buffer.from('😀');
  for(let index=1;index<emoji.length;index++) {
    const {invoke}=run([out(emoji.subarray(0,index),false),err('\ufeff\ufffd\0'),out(emoji.subarray(index)),poll()]);
    assert.deepEqual(invoke(),{code:0,stdout:'😀',stderr:'\ufeff\ufffd\0'});
  }
  const cross=run([out([0xe4,0xb8]),err([0xad]),poll()]);
  assert.throws(cross.invoke,fault('process_invalid_utf8'));
  for(const bytes of [[0xff],[0x80],[0xc0,0x80],[0xed,0xa0,0x80],[0xf4,0x90,0x80,0x80],[0xe2,0x82]]) {
    const {invoke}=run([out(bytes),err(''),poll()]);
    assert.throws(invoke,fault('process_invalid_utf8'));
  }
});

test('已经替换坏字节的字符串不能证明严格UTF8，不能直接包装旧run字符串结果',()=>{
  const malformed=Buffer.from([0xff]).toString('utf8');
  const realReplacement=Buffer.from('\ufffd').toString('utf8');
  assert.equal(malformed,realReplacement);
  assert.deepEqual(validateCapturedValue({code:0,stdout:realReplacement,stderr:''}),{code:0,stdout:'\ufffd',stderr:''});
  assert.throws(run([out([0xff]),err(''),poll()]).invoke,fault('process_invalid_utf8'));
});

test('非零退出默认失败，显式允许才交付；code为i32，不接纳signal/null伪装成功',()=>{
  assert.throws(run([out(secret),err(secret),poll(7)]).invoke,fault('process_exit_nonzero'));
  assert.deepEqual(run([out(secret),err(''),poll(-1)],{allow_non_zero:true}).invoke(),{code:-1,stdout:secret,stderr:''});
  assert.throws(run([out(''),err(''),poll(4294967295)]).invoke,fault('process_wait_failed'));
  assert.throws(run([out(''),err(''),poll(null)]).invoke,fault('process_read_failed'));
});

test('退出不等于管道EOF，后代持管道到deadline仍明确超时',()=>{
  const {backend,invoke}=run([out(secret,false),err(''),poll(0,1n),poll(0,3n)],{timeout_ms:3});
  assert.throws(invoke,fault('process_timeout'));
  assert.deepEqual(backend.records.map(item=>item.kind),['start','finish']);
  assert.throws(run([out(''),err(''),poll(0,3n)],{timeout_ms:3}).invoke,fault('process_timeout'));
});

test('显式时钟倒退/溢出和取消分别保留开始前后效果；没有实际sleep或进程',()=>{
  const pre=run([],{},{clock:(1n<<64n)-1n});
  assert.throws(pre.invoke,fault('process_clock_error','not_started'));assert.equal(pre.backend.records.length,0);
  const back=run([out('',false),err('',false),poll(null,2n),poll(null,1n)]);
  assert.throws(back.invoke,fault('process_clock_error'));
  const early=run([],{},{cancelBefore:true});
  assert.throws(early.invoke,fault('cancelled','not_started'));assert.equal(early.backend.records.length,0);
  const late=run([out(secret,false),{type:'cancel'}]);
  assert.throws(late.invoke,fault('cancelled'));assert.deepEqual(late.backend.records.map(item=>item.kind),['start','finish']);
});

test('启动失败不交付输出；读取/等待失败不泄漏正文，cleanup失败覆盖其他错误',()=>{
  const start=run([],{},{startError:'process_start_failed'});
  assert.throws(start.invoke,fault('process_start_failed','not_started'));
  assert.equal(start.backend.records.length,1);
  for(const code of ['process_read_failed','process_wait_failed']) {
    assert.throws(run([out(secret,false),{type:'failure',code}]).invoke,fault(code));
  }
  const cleanup=run([out([0xff]),err(''),poll()],{},{cleanupError:true});
  assert.throws(cleanup.invoke,fault('process_cleanup_failed'));
});

test('单独kill/reap直接子进程不足以返回成功，必须确认私有Job及父进程收尾',()=>{
  for(const flags of [{privateJobEmpty:false},{parentReaped:false},{privateJobEmpty:false,parentReaped:false}]) {
    const {invoke}=run([out(secret),err(''),poll()],{},flags);
    assert.throws(invoke,fault('process_cleanup_failed'));
  }
});

test('JSON结果严格字段、Unicode、容量与code验证，丢弃后端正文错误字段',()=>{
  for(const value of [null,[],{}, {code:0,stdout:'',stderr:'',extra:secret},
    {code:null,stdout:'',stderr:''},{code:1.1,stdout:'',stderr:''},{code:0,stdout:'\ud800',stderr:''},
    {code:0,stdout:'x'.repeat(65536),stderr:'x'},{code:0,stdout:1,stderr:''}]) {
    assert.throws(()=>validateCapturedValue(value),fault('invalid_response'));
  }
});

test('后端未知启动阶段及恶意错误getter保守unknown并收尾，不回显异常正文',()=>{
  const backend=new RecordingCaptureBackend([]);
  const poison=Object.defineProperty(Error(secret),'code',{get(){throw Error(secret);}});
  backend.start=function(){this.records.push({kind:'start'});throw poison;};
  assert.throws(()=>runRecordedCapture(request,backend),fault('process_capture_failed'));
  assert.deepEqual(backend.records.map(record=>record.kind),['start','finish']);
  const input={...request};
  Object.defineProperty(input,'program',{get(){throw poison;}});
  assert.throws(()=>normalizeCaptureWire(input),fault('invalid_arguments','not_started'));
});
