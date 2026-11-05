import assert from 'node:assert/strict';
import {mkdirSync,mkdtempSync,readdirSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import {checkPath,inspectHistory} from '../tools/script-sdk-check/inspect-clipboard-history.mjs';

test('只读候选扫描不得读取第1001个目录项',()=>{
  const repo=resolve(dirname(fileURLToPath(import.meta.url)),'..');
  checkPath(repo,{directory:true});
  const temp=join(repo,'.tmp');
  if(!checkPath(temp,{directory:true,allowMissingFinal:true}))mkdirSync(temp);
  const root=mkdtempSync(join(temp,'history-inspector-limit-synthetic-'));
  checkPath(root,{directory:true});
  // 只在本次新建目录创建固定名称普通文件；wx 不覆盖现有对象。
  for(let index=0;index<1001;index++)writeFileSync(join(root,`unrelated-${index}.txt`),'synthetic',{flag:'wx'});
  const history=join(root,'history.json');
  writeFileSync(history,JSON.stringify({version:1,lastId:'0',revision:'0',items:[]}),{flag:'wx'});
  writeFileSync(history+'.lock','OmniTrigger.ClipboardHistory.Lock.v1\n0\n',{flag:'wx'});
  const before=readdirSync(root).sort();
  const result=inspectHistory(history,{scanTemporaries:true});
  assert.equal(result.history.state,'valid');
  assert.equal(result.lock.state,'fresh');
  assert.equal(result.temporaries.scannedEntries,1000);
  assert.equal(result.temporaries.complete,false);
  assert.equal(result.temporaries.candidates.length,0);
  assert.equal(result.readOnly,true);
  assert.deepEqual(readdirSync(root).sort(),before);
});
