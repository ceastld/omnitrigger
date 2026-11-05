import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {linkSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, symlinkSync, writeFileSync} from 'node:fs';
import {dirname, isAbsolute, join, relative, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import {checkPath, inspectDocument, inspectHistory, unsafeAttributes} from '../tools/script-sdk-check/inspect-clipboard-history.mjs';

const repo = resolve(dirname(fileURLToPath(import.meta.url)),'..');
const syntheticText = '合成私密正文_DO_NOT_PRINT_🦀\r\n  ';
const magic = 'OmniTrigger.ClipboardHistory.Lock.v1\n';
const valid = () => ({version:1,lastId:'2',revision:'3',items:[{id:'1',text:syntheticText,createdAt:'0'},{id:'2',text:'',createdAt:'1'}]});
const bytes = value => Buffer.from(JSON.stringify(value));

function noBody(result) {
  const serialized = JSON.stringify(result);
  assert.equal(serialized.includes(syntheticText),false);
  assert.equal(serialized.includes('DO_NOT_PRINT'),false);
  assert.equal(serialized.includes('合成私密正文'),false);
  assert.equal(Object.hasOwn(result,'buffer'),false);
}
function fixture() {
  const temp = join(repo,'.tmp');
  checkPath(temp,{directory:true});
  const root = mkdtempSync(join(temp,'history-inspector-synthetic-'));
  checkPath(root,{directory:true});
  const history = join(root,'history.json');
  const owned = path => {
    const name = relative(root,path);
    assert(name !== '' && name !== '..' && !name.startsWith('../') && !name.startsWith('..\\') && !isAbsolute(name));
    checkPath(dirname(path),{directory:true});
    checkPath(path,{allowMissingFinal:true});
    return path;
  };
  return {root,history,
    put(name,contents) { const path = owned(join(root,name)); writeFileSync(path,contents,{flag:'wx'}); return path; },
    snapshot() {
      checkPath(root,{directory:true});
      return readdirSync(root).sort().map(name => {
        const path = join(root,name);
        checkPath(path);
        return [name,createHash('sha256').update(readFileSync(path)).digest('hex')];
      });
    },
  };
}

test('合法模型只返回元数据，u64字符串、空文本和Unicode完整接受', () => {
  const result = inspectDocument(bytes(valid()));
  assert.deepEqual(result,{state:'valid',schemaVersion:1,lastId:'2',revision:'3',itemCount:2,totalUtf16Units:syntheticText.length,oldestId:'1',newestId:'2'});
  noBody(result);
  const highest = {version:1,lastId:'18446744073709551615',revision:'18446744073709551615',items:[{id:'18446744073709551615',text:'😀',createdAt:'18446744073709551615'}]};
  assert.equal(inspectDocument(bytes(highest)).state,'valid');
});

test('损坏、未知字段、重复转义字段、非UTF8、BOM及复杂结构均给固定原因', () => {
  const cases = [
    [Buffer.from('{'+syntheticText),'invalid_json'],
    [Buffer.from([0xff,0xfe]),'invalid_utf8'],
    [Buffer.concat([Buffer.from([0xef,0xbb,0xbf]),bytes(valid())]),'invalid_json'],
    [Buffer.from('{"version":1,"version":2,"lastId":"0","revision":"0","items":[]}'),'duplicate_field'],
    [Buffer.from(String.raw`{"version":1,"lastId":"0","revision":"0","items":[],"\u0076ersion":1}`),'duplicate_field'],
    [bytes({...valid(),extra:syntheticText}),'document_fields'],
    [Buffer.from('{"version":1.0,"lastId":"0","revision":"0","items":[]}'),'unsupported_version'],
    [bytes([1,'0','0',[]]),'document_fields'],
    [Buffer.from('['.repeat(18)+'0'+']'.repeat(18)),'json_complexity_limit'],
    [Buffer.alloc(8*1024*1024+1),'storage_limit'],
  ];
  for (const [contents,reason] of cases) {
    const result = inspectDocument(contents);
    assert.deepEqual(result,{state:'corrupt',reason});
    noBody(result);
  }
});

test('文本、计数、顺序和容量边界与存储模型一致', () => {
  const mutations = [
    document => { document.lastId='01'; },
    document => { document.revision='1'; },
    document => { document.items[1].id='1'; },
    document => { document.items[1].id='3'; },
    document => { document.items[0].createdAt='-1'; },
    document => { document.items[0].text='\0'; },
    document => { document.items[0].text='\ud800'; },
    document => { document.items[0].text='😀'.repeat(32769); },
    document => { document.items[0].unknown=syntheticText; },
  ];
  for (const mutate of mutations) {
    const document = valid(); mutate(document);
    const result = inspectDocument(bytes(document));
    assert.equal(result.state,'corrupt'); noBody(result);
  }
  const capacity = {version:1,lastId:'17',revision:'17',items:Array.from({length:16},(_,index)=>({id:String(index+1),text:'x'.repeat(65536),createdAt:'0'}))};
  assert.equal(inspectDocument(bytes(capacity)).totalUtf16Units,1048576);
  capacity.items.push({id:'17',text:'x',createdAt:'0'});
  assert.equal(inspectDocument(bytes(capacity)).reason,'total_units_limit');
});

test('属性掩码逐个拒绝Offline、Recall和Reparse', () => {
  for (const attribute of [0x400,0x1000,0x40000,0x400000]) assert.equal(unsafeAttributes(attribute),true);
  for (const attribute of [0,0x20,0x80]) assert.equal(unsafeAttributes(attribute),false);
});

test('真实合成目录只读检查有效正文、锁和完整/损坏临时候选，不清理任何文件', () => {
  const f = fixture();
  f.put('history.json',bytes(valid()));
  f.put('history.json.lock',magic+'1\n');
  f.put('history.json.123.1.tmp',bytes({...valid(),lastId:'3',revision:'4'}));
  f.put('history.json.123.2.tmp',syntheticText);
  f.put('history.json.unrelated.tmp',syntheticText);
  const before = f.snapshot();
  const result = inspectHistory(f.history,{scanTemporaries:true});
  assert.equal(result.readOnly,true);
  assert.equal(result.consistency,'unlocked_observation');
  assert.equal(result.writerLiveness,'unknown');
  assert.equal(result.history.state,'valid');
  assert.equal(result.lock.state,'initialized');
  assert.deepEqual(result.temporaries.candidates.map(item=>item.state).sort(),['corrupt','valid']);
  assert.equal(result.temporaries.candidates.length,2);
  assert.deepEqual(result.recovery,{automatic:false,mayOverwrite:false,mayDelete:false,mayResetIds:false});
  noBody(result);
  assert.deepEqual(f.snapshot(),before);
  assert.equal(inspectHistory(f.history).temporaries.requested,false);
});

test('正文消失且锁已初始化时保留ID证据；未知锁不被修复或误判高位字节', () => {
  const missing = fixture();
  missing.put('history.json.lock',magic+'1\n');
  const before = missing.snapshot();
  const report = inspectHistory(missing.history);
  assert.equal(report.history.state,'missing');
  assert(report.findings.includes('missing_initialized_history_preserve_sidecar'));
  assert.deepEqual(missing.snapshot(),before);
  const unknown = fixture();
  unknown.put('history.json',bytes(valid()));
  const marker = Buffer.from(magic+'1\n'); marker[0] |= 0x80;
  unknown.put('history.json.lock',marker);
  const saved = unknown.snapshot();
  assert.equal(inspectHistory(unknown.history).lock.state,'corrupt');
  assert.deepEqual(unknown.snapshot(),saved);
});

test('临时文件扫描有界且不扫描递归目录', () => {
  const f = fixture();
  f.put('history.json',bytes(valid()));
  for (let index=1;index<=9;index++) f.put(`history.json.123.${index}.tmp`,bytes(valid()));
  const report = inspectHistory(f.history,{scanTemporaries:true});
  assert.equal(report.temporaries.candidates.length,8);
  assert.equal(report.temporaries.complete,false);
  noBody(report);
});

test('只接受显式绝对路径，并拒绝合成链接、硬链接和云盘名称而不读取目标', () => {
  assert.equal(inspectHistory('relative.json').reason,'explicit_absolute_file_required');
  const f = fixture();
  const target = f.put('target.json',bytes(valid()));
  const alias = join(f.root,'alias.json');
  checkPath(alias,{allowMissingFinal:true});
  const targetDirectory=join(f.root,'target-directory');
  mkdirSync(targetDirectory);
  symlinkSync(targetDirectory,alias,process.platform==='win32'?'junction':'dir');
  const report = inspectHistory(alias);
  assert.equal(report.history.state,'unavailable');
  assert(['link_rejected','unavailable_attributes_rejected'].includes(report.history.reason));
  const hard = join(f.root,'hard.json');
  checkPath(hard,{allowMissingFinal:true}); linkSync(target,hard);
  assert.equal(inspectHistory(hard).history.reason,'hard_link_rejected');
  const cloud = inspectHistory(join(f.root,'OneDrive-synthetic','history.json'));
  assert.equal(cloud.state,'unavailable');
  assert.equal(cloud.reason,'cloud_path_rejected');
  noBody(report); noBody(cloud);
});
