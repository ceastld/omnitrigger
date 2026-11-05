// 全部使用本次临时合成工程，作者模块含抛错初始化器，检查过程不得执行它们。
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,readFileSync,writeFileSync,unlinkSync,linkSync} from 'node:fs';
import {basename,dirname,join,resolve} from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import {checkProjectTypes} from './check-project-types.mjs';

const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const parent=join(repo,'.tmp/continuous-actions/sdk');mkdirSync(parent,{recursive:true});
const sdk=readFileSync(join(repo,'crates/omnitrigger-actions/runtime/script-api.d.ts'),'utf8');
function fixture() {
  const root=mkdtempSync(join(parent,'project-check-'));
  mkdirSync(join(root,'sdk'));mkdirSync(join(root,'lib'));
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,id:'synthetic.project',actions:[{id:'run',runtime:'script',entry:'main.mjs'}]}));
  writeFileSync(join(root,'sdk/api.d.ts'),sdk);
  writeFileSync(join(root,'jsconfig.json'),JSON.stringify({compilerOptions:{strict:true,target:'ES2020',module:'ESNext',moduleResolution:'Bundler',lib:['ES2020']},include:['**/*.mjs','sdk/**/*.d.ts']}));
  writeFileSync(join(root,'lib/text.mjs'),'throw Error("INITIALIZER MUST NOT RUN");\n/** @param {string} text */\nexport function clean(text) { return text.trim(); }\n');
  writeFileSync(join(root,'main.mjs'),'/// <reference path="./sdk/api.d.ts" />\nimport {clean} from "./lib/text.mjs";\n/** @type {ScriptMain} */\nexport function main(ctx) { const result=ctx.clipboard.history.list(); return {count:result.items.length,value:clean(" synthetic ")}; }\n');
  return root;
}

test('检查既有入口与相对依赖，保留源码并且不执行模块初始化器',()=>{
  const root=fixture();const before=readFileSync(join(root,'main.mjs'));
  const result=checkProjectTypes(root);
  assert.equal(result.passed,true,JSON.stringify(result.diagnostics));
  assert(result.checkedFiles.includes('lib/text.mjs'));
  assert(result.checkedFiles.includes('sdk/api.d.ts'));
  assert.equal(result.runtimeExecuted,false);assert.equal(result.filesModified,false);
  assert.deepEqual(readFileSync(join(root,'main.mjs')),before);
});

test('错误参数返回实际文件行列及TypeScript错误，而不运行App',()=>{
  const root=fixture();const file=join(root,'main.mjs');
  writeFileSync(file,'/// <reference path="./sdk/api.d.ts" />\n/** @type {ScriptMain} */\nexport function main(ctx) { return ctx.clipboard.history.get({id:1}); }\n');
  const result=checkProjectTypes(root);
  assert.equal(result.passed,false);
  assert(result.diagnostics.some(item=>item.file==='main.mjs'&&item.line===3&&item.code===2322));
});

test('外部导入仅报告无法解析，工程外文件不列入读取集合',()=>{
  const root=fixture();
  const outside=basename(root)+'.outside.mjs';
  writeFileSync(join(root,'main.mjs'),`import {value} from "../${outside}";\nexport function main() { return value; }`);
  writeFileSync(join(dirname(root),outside),'throw Error("EXTERNAL INITIALIZER MUST NOT RUN");export const value=1;', {flag:'wx'});
  const result=checkProjectTypes(root);
  assert.equal(result.passed,false);
  assert(result.diagnostics.some(item=>item.code===2307));
  assert(result.checkedFiles.every(file=>!file.startsWith('../')));
});

test('缺SDK、extends和云盘命名路径都明确拒绝，不擅自补写',()=>{
  const root=fixture();unlinkSync(join(root,'sdk/api.d.ts'));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_sdk_missing');
  writeFileSync(join(root,'sdk/api.d.ts'),sdk);
  writeFileSync(join(root,'jsconfig.json'),' {"extends":"../../outside.json"} ');
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_config_unsupported');
  writeFileSync(join(root,'jsconfig.json'),' {"compilerOptions":null} ');
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_config_invalid');
  const cloud=join(root,'OneDrive-Synthetic');mkdirSync(cloud);
  assert.throws(()=>checkProjectTypes(cloud),error=>error.code==='project_cloud_path');
});

test('源文件上限先于解析拒绝，清单中的越界入口也拒绝',()=>{
  const root=fixture();writeFileSync(join(root,'main.mjs'),'x'.repeat(128*1024+1));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_file_limit');
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[{runtime:'script',entry:'../external.mjs'}]}));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_entry_invalid');
});

test('无效UTF8与硬链接在内容检查前明确拒绝，不静默替换字节',()=>{
  const root=fixture();writeFileSync(join(root,'main.mjs'),Buffer.from([0xff,0xfe,0x61]));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_utf8_invalid');
  writeFileSync(join(root,'main.mjs'),'export function main() { return null; }');
  linkSync(join(root,'main.mjs'),join(root,'same-file.mjs'));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_link_path');
});

test('CLI缺参数返回结构化失败和退出码，不尝试寻找工程',()=>{
  const result=spawnSync(process.execPath,[join(repo,'tools/script-sdk-check/check-project-types.mjs')],
    {encoding:'utf8',windowsHide:true,timeout:10000});
  assert.ifError(result.error);assert.equal(result.status,1);
  const output=JSON.parse(result.stderr);
  assert.equal(output.error.code,'project_directory_required');
  assert.equal(output.runtimeExecuted,false);assert.equal(output.filesModified,false);
});
