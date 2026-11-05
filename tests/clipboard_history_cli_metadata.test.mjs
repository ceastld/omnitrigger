import assert from 'node:assert/strict';
import {mkdirSync,mkdtempSync,readFileSync,symlinkSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import {classifyMetadataCheck,selfTest,verifyOrdinaryPaths} from '../tools/script-sdk-check/check-clipboard-history.mjs';

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

test('单路径数组在Windows PowerShell 5下仍按单个路径处理',()=>{
  const root=fixture();
  const report=verifyOrdinaryPaths([root]);
  assert.equal(report.passed,true);
  assert.equal(report.metadataProcesses,process.platform==='win32'?1:0);
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
