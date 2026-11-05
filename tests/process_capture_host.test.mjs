// 真实宿主API调用；只执行明确自有夹具，所有终态只包含长度、布尔值和错误代码。
import assert from 'node:assert/strict';
import {mkdirSync,writeFileSync} from 'node:fs';
import {join} from 'node:path';
import {before,test} from 'node:test';
import {configuration,createCase,runHost,warmup} from '../tools/script-sdk-check/process-capture-test-support.mjs';

let config;
before(()=>{config=configuration();warmup(config);});
const script=`
const normalize=path=>{
  const slash=String.fromCharCode(92);
  let value=path.split('/').join(slash);
  if(value.startsWith(slash+slash+'?'+slash))value=value.slice(4);
  while(value.endsWith(slash))value=value.slice(0,-1);
  return value.toLowerCase();
};
try {
  const result=ctx.process.capture(ctx.input.program,ctx.input.args,ctx.input.options);
  const cwdMatches=ctx.input.expectedCwd===undefined?null:normalize(result.stdout)===normalize(ctx.input.expectedCwd);
  return {captured:true,code:result.code,stdoutUnits:result.stdout.length,stderrUnits:result.stderr.length,
    stdoutMatches:ctx.input.expectedOut===undefined?null:result.stdout===ctx.input.expectedOut,
    stderrMatches:ctx.input.expectedErr===undefined?null:result.stderr===ctx.input.expectedErr,cwdMatches};
} catch(error) {
  return {captured:false,code:error.code,effectState:error.effectState,method:error.method,
    partialOutput:error.stdout!==undefined||error.stderr!==undefined,
    messageRedacted:!String(error.message).includes('controlled output')};
}
`;
function execute(label,args,options={},expected={}) {
  const own=createCase(config,label);
  const output=runHost(config,own.root,script,{program:config.fixture,args,options,...expected});
  writeFileSync(join(own.root,'host-result.json'),JSON.stringify({engine:config.engine,...output}),{flag:'wx'});
  assert(!JSON.stringify(output).includes('controlled output'));
  assert.equal(Object.hasOwn(output,'stdout'),false);assert.equal(Object.hasOwn(output,'stderr'),false);
  return output;
}
function failed(code) {
  return {captured:false,code,effectState:'unknown',method:'process.capture',partialOutput:false,messageRedacted:true};
}
test('stdout和stderr同时采集，合计65536字节完整但终态不回显正文',()=>{
  assert.deepEqual(execute('dual',['dual','32768','32768'],{},
    {expectedOut:'a'.repeat(32768),expectedErr:'b'.repeat(32768)}),
  {captured:true,code:0,stdoutUnits:32768,stderrUnits:32768,stdoutMatches:true,stderrMatches:true,cwdMatches:null});
});
test('超出共享容量没有部分回执',()=>{
  assert.deepEqual(execute('overflow',['dual','32769','32768']),failed('process_output_too_large'));
});
test('非法编码不替换为字符，不交付部分输出',()=>{
  assert.deepEqual(execute('utf8',['invalid-utf8']),failed('process_invalid_utf8'));
});
test('超时明确失败并保留开始后效果',()=>{
  assert.deepEqual(execute('timeout',['sleep','1000'],{timeoutMs:10}),failed('process_timeout'));
});
test('非零默认拒绝，显式允许才交付且只返回摘要',()=>{
  assert.deepEqual(execute('nonzero',['exit','7']),failed('process_exit_nonzero'));
  assert.deepEqual(execute('allowed',['exit','7'],{allowNonZero:true},{expectedOut:'controlled output',expectedErr:''}),
    {captured:true,code:7,stdoutUnits:17,stderrUnits:0,stdoutMatches:true,stderrMatches:true,cwdMatches:null});
});
test('省略cwd继承明确宿主目录，显式cwd只影响本次目标',()=>{
  for(const explicit of [false,true]) {
    const own=createCase(config,explicit?'explicit-cwd':'default-cwd');
    const cwd=explicit?join(own.root,'显式 cwd'):own.root;
    if(explicit)mkdirSync(cwd);
    const result=runHost(config,own.root,script,{program:config.fixture,args:['cwd'],options:explicit?{cwd}:{},expectedCwd:cwd});
    assert.equal(result.captured,true);assert.equal(result.cwdMatches,true);
    assert.equal(result.stderrUnits,0);assert.equal(Object.hasOwn(result,'stdout'),false);
    writeFileSync(join(own.root,'host-result.json'),JSON.stringify({engine:config.engine,...result}),{flag:'wx'});
  }
});
