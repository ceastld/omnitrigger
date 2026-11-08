import assert from 'node:assert/strict';
import {mkdirSync,mkdtempSync,readFileSync,symlinkSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import {classifyMetadataCheck,metadataProgram,selfTest,verifyOrdinaryPaths} from '../tools/script-sdk-check/check-clipboard-history.mjs';

const repo=resolve(dirname(fileURLToPath(import.meta.url)),'..');
function fixture() {
  const parent=join(repo,'.tmp');
  verifyOrdinaryPaths([parent]);
  const root=mkdtempSync(join(parent,'history-cli-metadata-synthetic-'));
  verifyOrdinaryPaths([root]);
  return root;
}

test('固定错误分类保留状态、超时与stderr摘要，绝不打印stderr正文',async()=>{
  const result=await selfTest();
  assert.equal(result.metadataClassificationCases,8);
  assert.equal(result.appStarted,false);
  const raw='合成正文_NOT_FOR_DIAGNOSTICS';
  const error=classifyMetadataCheck({status:null,error:{code:'ETIMEDOUT'},stderr:raw},1);
  assert.equal(error.classification,'metadata_timeout');
  assert.equal(error.errorCode,'ETIMEDOUT');
  assert.equal(error.stderrSummary.bytes,Buffer.byteLength(raw));
  assert(!JSON.stringify(error).includes(raw));
});

test('一个批次去重祖先并处理多个现有/不存在合成路径，只启动一次元数据进程',()=>{
  const root=fixture();
  const folder=join(root,'folder'); mkdirSync(folder);
  const file=join(folder,'file.txt'); writeFileSync(file,'合成元数据夹具',{flag:'wx'});
  const before=readFileSync(file);
  const result=verifyOrdinaryPaths([root,folder,file,file,join(folder,'absent.json')]);
  assert.equal(result.passed,true);
  assert.equal(result.metadataProcesses,process.platform==='win32'?1:0);
  assert.equal(result.requestedPaths,5);
  assert.deepEqual(readFileSync(file),before);
});

test('单路径数组在Windows元数据程序下仍按单个路径处理',()=>{
  const root=fixture();
  const report=verifyOrdinaryPaths([root]);
  assert.equal(report.passed,true);
  assert.equal(report.metadataProcesses,process.platform==='win32'?1:0);
});

test('元数据程序由显式本地路径选择，未配置保留原默认且无隐式回退',()=>{
  assert.equal(metadataProgram({}), 'powershell.exe');
  const path='C:\\Program Files\\PowerShell\\7\\pwsh.exe';
  assert.equal(metadataProgram({OMNITRIGGER_METADATA_POWERSHELL:path}),path);
  for(const value of ['', 'pwsh.exe', ' C:\\pwsh.exe', 'C:\\pwsh.exe\n', 'C:\\pwsh.ps1', '\\\\server\\pwsh.exe',
    'C:\\OneDrive-synthetic\\pwsh.exe', 'C:\\Dropbox\\pwsh.exe', 'C:\\iCloudDrive\\pwsh.exe', 42]) {
    assert.throws(()=>metadataProgram({OMNITRIGGER_METADATA_POWERSHELL:value}),error=>error.stage==='invalid_metadata_program');
  }
});

test('保留云盘名称与链接保护，junction只指向自身合成目录',()=>{
  const root=fixture();
  assert.throws(()=>verifyOrdinaryPaths([join(root,'OneDrive-synthetic','absent')]),error=>error.stage==='unsafe_cloud_path');
  const target=join(root,'target'); mkdirSync(target);
  const alias=join(root,'alias');
  symlinkSync(target,alias,process.platform==='win32'?'junction':'dir');
  assert.throws(()=>verifyOrdinaryPaths([alias]),error=>{
    assert(['unsafe_metadata_attributes','unsafe_link'].includes(error.stage));
    if(process.platform==='win32') {
      assert.equal(error.detail.metadata.exitCode,7);
      assert(error.detail.metadata.blockedAttributes&0x400);
    }
    return true;
  });
});


test('metadata传输诊断保留有界PID与stdout长度，不把部分回执当成功或回显正文',()=>{
  const raw='合成_stdout_不可回显😀',stderr='合成_stderr_不可回显';
  const result=classifyMetadataCheck({status:null,pid:1234,signal:'SIGTERM',error:{code:'ETIMEDOUT',message:raw},stdout:raw,stderr},11);
  assert.equal(result.classification,'metadata_timeout');assert.equal(result.childPid,1234);
  assert.deepEqual(result.stdoutSummary,{present:true,bytes:Buffer.byteLength(raw),category:'suppressed_powershell_stdout'});
  assert.equal(result.stderrSummary.bytes,Buffer.byteLength(stderr));
  assert(!JSON.stringify(result).includes(raw));assert(!JSON.stringify(result).includes(stderr));
  for(const pid of [undefined,null,0,-1,1.5,NaN,Infinity,0x100000000,'1234',raw]){
    const bounded=classifyMetadataCheck({status:null,pid,error:{code:'ENOENT'},stdout:''},11);
    assert.equal(bounded.childPid,null);assert.equal(bounded.stdoutSummary.present,false);assert.equal(bounded.stdoutSummary.bytes,0);
  }
  const partial=classifyMetadataCheck({status:null,pid:1234,signal:'SIGTERM',error:{code:'ETIMEDOUT'},stdout:'{"classification":"checked","checkedCount":11}'},11);
  assert.equal(partial.classification,'metadata_timeout');assert.equal(partial.stdoutSummary.present,true);
  const checked=classifyMetadataCheck({status:0,pid:1234,stdout:'{"classification":"checked","checkedCount":11}',stderr:''},11);
  assert.equal(checked.classification,'checked');assert.equal(checked.childPid,1234);
});
