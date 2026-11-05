// 实际进程验收的显式自有路径与协议支持；导入本模块不会启动任何程序。
import assert from 'node:assert/strict';
import {spawn,spawnSync} from 'node:child_process';
import {randomBytes} from 'node:crypto';
import {lstatSync,mkdtempSync,realpathSync,writeFileSync} from 'node:fs';
import {isAbsolute,join,relative,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
// 只复用稳定的元数据保护函数，不调用历史CLI/App验收入口。
import {verifyOrdinaryPaths} from './check-clipboard-history.mjs';

export const fixtureSignature='omnitrigger-process-capture-fixture-v1';
export const pause=milliseconds=>new Promise(done=>setTimeout(done,milliseconds));
function local(value) {
  return typeof value==='string'&&value.length>0&&value===value.trim()&&isAbsolute(value)
    &&/^(?:[A-Za-z]:\\|\\\\\?\\[A-Za-z]:\\)/.test(value);
}
export function configuration(env=process.env) {
  assert.equal(process.platform,'win32','实际capture验收只在Windows显式运行');
  const names={host:'OMNITRIGGER_CAPTURE_HOST',fixture:'OMNITRIGGER_CAPTURE_FIXTURE',parent:'OMNITRIGGER_CAPTURE_CASE_ROOT'};
  const values={};
  for(const [key,name] of Object.entries(names)) {
    assert(local(env[name]),`必须提供明确本地绝对路径：${name}`);
    values[key]=resolve(env[name]);
  }
  assert(['boa','quickjs'].includes(env.OMNITRIGGER_CAPTURE_ENGINE),'必须明确OMNITRIGGER_CAPTURE_ENGINE为boa或quickjs');
  verifyOrdinaryPaths(Object.values(values));
  for(const key of ['host','fixture']) {
    assert(values[key].toLowerCase().endsWith('.exe')&&lstatSync(values[key]).isFile(),`${names[key]}必须是已构建的普通EXE`);
    assert.equal(relative(values[key],realpathSync(values[key])),'','不接受EXE路径别名');
  }
  assert(lstatSync(values.parent).isDirectory(),'CASE_ROOT必须已显式创建');
  assert.equal(relative(values.parent,realpathSync(values.parent)),'','不接受测试父目录别名');
  return {...values,engine:env.OMNITRIGGER_CAPTURE_ENGINE};
}
export function createCase(config,label) {
  verifyOrdinaryPaths([config.parent]);
  const root=mkdtempSync(join(config.parent,'capture-case-'));
  verifyOrdinaryPaths([root]);
  const token=randomBytes(16).toString('hex');
  writeFileSync(join(root,'case-owner.json'),JSON.stringify({schemaVersion:1,token,label,owner:process.pid}),{flag:'wx'});
  return {root,token};
}
function resultJson(result,label) {
  assert.ifError(result.error);
  assert.equal(result.status,0,`${label}未正常结束；不记录程序正文`);
  let parsed;
  try {parsed=JSON.parse(result.stdout);}catch {assert.fail(`${label}未返回单一JSON回执`);}
  return parsed;
}
export function runHost(config,root,script,input,timeout=20000) {
  verifyOrdinaryPaths([config.host,root]);
  const response=resultJson(spawnSync(config.host,[],{cwd:root,windowsHide:true,encoding:'utf8',
    input:JSON.stringify({script,input}),timeout,maxBuffer:128*1024}), '受控宿主');
  assert.equal(response.schema_version,1);
  assert.equal(response.ok,true,'受控脚本入口未完成；不输出错误或采集正文');
  return response.result;
}
export function warmup(config) {
  const own=createCase(config,'warmup');
  const descriptor=resultJson(spawnSync(config.host,['--describe-runtime'],{windowsHide:true,encoding:'utf8',timeout:60000,maxBuffer:4096}), '宿主能力描述');
  assert.equal(descriptor.product,'omnitrigger-script-host');
  assert.equal(descriptor.engine,config.engine,'宿主引擎与显式环境不符');
  assert.equal(descriptor.api_version,1);assert.equal(descriptor.protocol_schema,1);
  const fixture=spawnSync(config.fixture,['warmup'],{cwd:own.root,windowsHide:true,encoding:'utf8',timeout:60000,maxBuffer:4096});
  assert.ifError(fixture.error);assert.equal(fixture.status,0,'夹具预热失败');
  assert.equal(fixture.stdout,fixtureSignature,'必须使用本测试的自有夹具');
  const script='const value=ctx.process.capture(ctx.input.program,["warmup"],{timeoutMs:10000});return {ready:value.code===0&&value.stdout===ctx.input.signature&&value.stderr===""};';
  assert.deepEqual(runHost(config,own.root,script,{program:config.fixture,signature:fixtureSignature}),{ready:true});
  return {engine:config.engine,fixtureWarmed:true,hostWarmed:true};
}
export function startOwnedHost(config,root,script,input) {
  verifyOrdinaryPaths([config.host,root]);
  const child=spawn(config.host,[],{cwd:root,windowsHide:true,stdio:['pipe','pipe','pipe']});
  let closed=false,spawnError;
  const bytes={stdout:0,stderr:0};
  child.stdout.on('data',chunk=>{bytes.stdout+=chunk.length;});
  child.stderr.on('data',chunk=>{bytes.stderr+=chunk.length;});
  child.stdin.on('error',()=>{});
  child.once('error',error=>{spawnError=error;});
  const close=new Promise(done=>child.once('close',(code,signal)=>{closed=true;done({code,signal});}));
  child.stdin.end(JSON.stringify({script,input}));
  return {child,bytes,get closed(){return closed;},get spawnError(){return spawnError;},
    async stop() {
      // 唯一会被终止的是此处spawn返回的ChildProcess，不根据夹具记录PID去kill。
      if(!closed&&!spawnError)child.kill();
      let timer;
      try {return await Promise.race([close,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('自有宿主未在期限内结束')),5000);})]);}
      finally {clearTimeout(timer);}
    }};
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  assert.deepEqual(process.argv.slice(2),['--warmup'],'只接受显式--warmup，不搜索或回退宿主');
  console.log(JSON.stringify(warmup(configuration())));
}
