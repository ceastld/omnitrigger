// 全部使用本次临时合成工程，作者模块含抛错初始化器，检查过程不得执行它们。
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,readFileSync,writeFileSync,unlinkSync,linkSync,rmdirSync} from 'node:fs';
import {basename,dirname,join,resolve,relative,isAbsolute} from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import ts from 'typescript';
import {checkProjectTypes} from './check-project-types.mjs';

const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const parent=join(repo,'.tmp/continuous-actions/sdk');mkdirSync(parent,{recursive:true});
const sdk=readFileSync(join(repo,'crates/omnitrigger-actions/runtime/script-types.d.ts'),'utf8');
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

function nodeFixture(mixed=false) {
  const root=fixture();
  for(const name of ['api.d.ts','clipboard-history-api.d.ts',...(mixed?['script-types.d.ts']:[])]) {
    writeFileSync(join(root,'sdk',name),readFileSync(join(repo,'crates/omnitrigger-actions/runtime',name)));
  }
  const nodeSource="throw Error('NODE INITIALIZER MUST NOT RUN');\n/** @param {import('./sdk/api.js').ActionContext} ctx */\nexport default async function main(ctx) { const target=await ctx.desktop.target();return {processName:target.processName}; }\n";
  writeFileSync(join(root,'node.mjs'),nodeSource);
  if(mixed)writeFileSync(join(root,'main.mjs'),"throw Error('SCRIPT INITIALIZER MUST NOT RUN');\n/** @type {ScriptMain} */\nexport function main(ctx) { return {processName:ctx.desktop.target().processName}; }\n");
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[
    ...(mixed?[{runtime:'script',entry:'main.mjs'}]:[]),{runtime:'node',entry:'node.mjs'}]}));
  writeFileSync(join(root,'jsconfig.json'),JSON.stringify({compilerOptions:{strict:true,skipLibCheck:false,
    target:'ES2020',module:'ESNext',moduleResolution:'Bundler',lib:['ES2020'],types:[]},
    files:mixed?['sdk/script-types.d.ts']:[]}));
  return {root,nodeSource};
}

test('Node索引的显式声明引用保持UTF8拒绝与包内父级引用语义',()=>{
  const {root}=nodeFixture();const declarations=join(root,'node_modules/@types/node');
  mkdirSync(declarations,{recursive:true});
  writeFileSync(join(root,'jsconfig.json'),JSON.stringify({compilerOptions:{strict:true,
    target:'ES2020',module:'ESNext',moduleResolution:'Bundler',lib:['ES2020'],types:['node']}}));
  writeFileSync(join(root,'node.mjs'),"throw Error('AUTHOR MUST NOT RUN');\nexport default function main() { return {pid:process.pid}; }\n");
  const index=join(declarations,'index.d.ts'),child=join(declarations,'globals.d.ts');
  writeFileSync(index,'/// <reference path="globals.d.ts" />\n');
  writeFileSync(child,'declare const process: {pid:number};\n');
  const valid=checkProjectTypes(root);assert.equal(valid.passed,true,JSON.stringify(valid));
  assert(valid.checkedFiles.includes('node_modules/@types/node/globals.d.ts'));
  writeFileSync(child,Buffer.from([0xff]));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_utf8_invalid'
    &&error.location?.file==='node_modules/@types/node/globals.d.ts');
  // 优化范围之外的合法包内引用继续交给原TS解析，而非新增拒绝。
  writeFileSync(index,'/// <reference path="../shared-node-global.d.ts" />\n');
  writeFileSync(join(declarations,'../shared-node-global.d.ts'),'declare const process: {pid:number};\n');
  const parent=checkProjectTypes(root);assert.equal(parent.passed,true,JSON.stringify(parent));
  assert(parent.checkedFiles.includes('node_modules/@types/shared-node-global.d.ts'));
  assert(!parent.checkedFiles.includes('node_modules/@types/node/globals.d.ts'));
});

test('统一Node检查使用真实SDK与同一受限工程，严格默认导出合同且不执行作者代码',()=>{
  const {root,nodeSource}=nodeFixture();const result=checkProjectTypes(root);
  assert.equal(result.passed,true,JSON.stringify(result));
  assert.deepEqual(result.entries,['node.mjs']);assert.deepEqual(result.moduleChecks,[]);
  assert.deepEqual(result.entryChecks,[]);assert.deepEqual(result.environmentDiagnostics,[]);
  assert.equal(result.nodeEntryChecks.length,1);assert.equal(result.nodeEntryChecks[0].passed,true);
  assert.equal(result.staticAnalysisComplete,true);assert.equal(result.runtimeExecuted,false);
  assert.equal(result.filesModified,false);assert.equal(readFileSync(join(root,'node.mjs'),'utf8'),nodeSource);
  writeFileSync(join(root,'node.mjs'),"throw Error('NODE INITIALIZER MUST NOT RUN');\nexport default 42;\n");
  const invalid=checkProjectTypes(root);assert.equal(invalid.passed,false);
  assert.deepEqual(invalid.diagnostics,[]);assert.equal(invalid.staticEnvironmentPassed,false);
  const error=invalid.environmentDiagnostics.find(item=>item.code==='node_entry_default_noncallable');
  assert(error);assert.equal(error.runtime,'node');assert.equal(error.entry,'node.mjs');
  assert.deepEqual([error.file,error.line,error.column],['node.mjs',2,1]);
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[{runtime:'node',entry:'node.cjs'}]}));
  writeFileSync(join(root,'node.cjs'),"throw Error('CJS INITIALIZER MUST NOT RUN');\nmodule.exports = function main() { return null; };\n");
  const cjs=checkProjectTypes(root);
  assert(cjs.nodeEntryChecks[0].diagnostics.some(item=>item.code==='node_entry_unresolved'&&item.severity==='warning'));
  assert.equal(cjs.staticAnalysisComplete,false);assert.deepEqual(cjs.skippedEntries,[]);
});

test('统一混合检查保留Script字段并核验Node默认导出，Promise误用仍有原数值定位',()=>{
  const {root,nodeSource}=nodeFixture(true);
  writeFileSync(join(root,'node.mjs'),"throw Error('NODE INITIALIZER MUST NOT RUN');\nexport function main() { return null; }\n");
  const missing=checkProjectTypes(root);assert.equal(missing.passed,false);
  assert.deepEqual(missing.entries,['main.mjs','node.mjs']);
  assert.equal(missing.moduleChecks.length,1);assert.equal(missing.moduleChecks[0].passed,true);
  assert.equal(missing.entryChecks[0].mainExport,'local_callable_declaration');
  assert(missing.environmentDiagnostics.some(item=>item.entry==='node.mjs'&&item.code==='node_entry_default_missing'));
  writeFileSync(join(root,'node.mjs'),nodeSource);
  const valid=checkProjectTypes(root);assert.equal(valid.passed,true,JSON.stringify(valid));
  const wrong=nodeSource.replace('await ctx.desktop.target()','ctx.desktop.target()');
  writeFileSync(join(root,'node.mjs'),wrong);
  const invalid=checkProjectTypes(root);assert.equal(invalid.passed,false);
  const error=invalid.diagnostics.find(item=>item.file==='node.mjs'&&item.code===2339);
  assert(error);assert.equal(error.line,3);
  assert.equal(error.column,wrong.split('\n')[2].indexOf('target.processName')+'target.'.length+1);
  assert.equal(invalid.entryChecks[0].passed,true);
});

test('统一Node环境按明确types解析，缺少Node依赖真实报错且混合包不注入Node全局',()=>{
  const {root}=nodeFixture();
  const config={compilerOptions:{strict:true,skipLibCheck:false,lib:['ES2020'],types:['node']}};
  writeFileSync(join(root,'jsconfig.json'),JSON.stringify(config));
  writeFileSync(join(root,'node.mjs'),"throw Error('NODE INITIALIZER MUST NOT RUN');\nexport default function main() { return {pid:process.pid}; }\n");
  const missing=checkProjectTypes(root);assert.equal(missing.passed,false);
  assert(missing.diagnostics.some(item=>item.code===2688&&item.message.includes('node')));
  assert(missing.diagnostics.some(item=>item.file==='node.mjs'&&item.line===2&&[2580,2591].includes(item.code)));
  const mixed=nodeFixture(true);writeFileSync(join(mixed.root,'jsconfig.json'),JSON.stringify(config));
  assert.throws(()=>checkProjectTypes(mixed.root),error=>error.code==='project_mixed_types_unsupported'
    &&error.location?.file==='jsconfig.json');
});

test('统一Node入口继续拒绝坏UTF8、源码超限与清单越界路径',()=>{
  const {root}=nodeFixture();const file=join(root,'node.mjs');
  writeFileSync(file,Buffer.from([0xff]));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_utf8_invalid'&&error.location?.file==='node.mjs');
  writeFileSync(file,'x'.repeat(128*1024+1));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_file_limit');
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[{runtime:'node',entry:'../external.mjs'}]}));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_entry_invalid');
});


function splitFixture() {
  const {root}=nodeFixture(true);
  unlinkSync(join(root,'jsconfig.json'));unlinkSync(join(root,'node.mjs'));unlinkSync(join(root,'main.mjs'));
  for(const name of ['node','script','shared'])mkdirSync(join(root,name));
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[
    {runtime:'node',entry:'node/main.mjs'},{runtime:'script',entry:'script/main.mjs'}]}));
  for(const runtime of ['node','script'])writeFileSync(join(root,runtime,'jsconfig.json'),JSON.stringify({
    compilerOptions:{strict:true,skipLibCheck:false,target:'ES2020',module:'ESNext',moduleResolution:'Bundler',lib:['ES2020'],types:[]},
    files:['main.mjs','../sdk/'+(runtime==='node'?'api.d.ts':'script-types.d.ts')]}));
  writeFileSync(join(root,'node/main.mjs'),"throw Error('NODE INITIALIZER MUST NOT RUN');\nimport {cleanText} from '../shared/text.mjs';\n/** @param {import('../sdk/api.js').ActionContext} ctx */\nexport default async function main(ctx) { const target=await ctx.desktop.target();return {text:cleanText(ctx.input.text),processName:target.processName}; }\n");
  writeFileSync(join(root,'script/main.mjs'),"throw Error('SCRIPT INITIALIZER MUST NOT RUN');\nimport {cleanText} from '../shared/text.mjs';\n/** @type {ScriptMain} */\nexport function main(ctx) { return {text:cleanText(ctx.input.text),processName:ctx.desktop.target().processName}; }\n");
  writeFileSync(join(root,'shared/text.mjs'),"throw Error('SHARED INITIALIZER MUST NOT RUN');\n/** @param {unknown} value @returns {string} */\nexport function cleanText(value) { return String(value ?? '').trim(); }\n");
  return root;
}

test('就近配置分别检查混合入口和共享模块，不把Node声明加入Script工程',()=>{
  const root=splitFixture();const result=checkProjectTypes(root);
  assert.equal(result.passed,true,JSON.stringify(result));assert.equal(result.projects.length,2);
  assert.deepEqual(result.entries,['node/main.mjs','script/main.mjs']);
  const node=result.projects.find(item=>item.config==='node/jsconfig.json');
  const script=result.projects.find(item=>item.config==='script/jsconfig.json');
  assert(node&&script);assert(node.checkedFiles.includes('sdk/api.d.ts'));
  assert(!node.checkedFiles.includes('sdk/script-types.d.ts'));
  assert(script.checkedFiles.includes('sdk/script-types.d.ts'));assert(!script.checkedFiles.includes('sdk/api.d.ts'));
  for(const project of [node,script])assert(project.checkedFiles.includes('shared/text.mjs'));
  assert.equal(result.nodeEntryChecks[0].passed,true);assert.equal(result.entryChecks[0].passed,true);
  assert.equal(result.runtimeExecuted,false);assert.equal(result.filesModified,false);
  writeFileSync(join(root,'shared/text.mjs'),"throw Error('SHARED INITIALIZER MUST NOT RUN');\n/** @param {unknown} value @returns {string} */\nexport function cleanText(value) { return 42; }\n");
  const invalid=checkProjectTypes(root);assert.equal(invalid.passed,false);
  const shared=invalid.diagnostics.filter(item=>item.file==='shared/text.mjs'&&item.code===2322);
  assert.deepEqual(new Set(shared.map(item=>item.config)),new Set(['node/jsconfig.json','script/jsconfig.json']));
  assert(shared.every(item=>item.line===3&&item.column>0));
});

test('就近配置损坏不回退父配置，声明仍受动作包边界而非子目录限制',()=>{
  const root=splitFixture();
  writeFileSync(join(root,'jsconfig.json'),JSON.stringify({compilerOptions:{strict:true,types:[]},files:['sdk/script-types.d.ts']}));
  const file=join(root,'script/jsconfig.json');
  writeFileSync(file,'{\n  "compilerOptions": {\n    "checkJs": }\n}\n');
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_config_invalid'
    &&error.location?.file==='script/jsconfig.json'&&error.location.line===3&&error.location.column===16);
  writeFileSync(file,Buffer.from([0xff]));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_utf8_invalid'
    &&error.location?.file==='script/jsconfig.json');
  writeFileSync(file,JSON.stringify({files:['../../outside.d.ts']}));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_config_unsupported'
    &&error.location?.file==='script/jsconfig.json');
  if(process.platform==='win32') {
    // 同一Windows配置不能借目录大小写不同被拆成两份，绕开混合环境拒绝。
    writeFileSync(join(root,'node/jsconfig.json'),JSON.stringify({compilerOptions:{types:['node']}}));
    writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[
      {runtime:'node',entry:'node/main.mjs'},{runtime:'script',entry:'NODE/main.mjs'}]}));
    assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_mixed_types_unsupported'
      &&error.location?.file==='node/jsconfig.json');
  }
});


test('就近配置允许根配置与子配置并存，项目读取记录不混入其他配置',()=>{
  const root=splitFixture();unlinkSync(join(root,'script/jsconfig.json'));
  writeFileSync(join(root,'jsconfig.json'),JSON.stringify({compilerOptions:{strict:true,skipLibCheck:false,
    target:'ES2020',module:'ESNext',moduleResolution:'Bundler',lib:['ES2020'],types:[]},files:['sdk/script-types.d.ts']}));
  const result=checkProjectTypes(root);assert.equal(result.passed,true,JSON.stringify(result));
  assert.deepEqual(result.projects.map(item=>item.config),['node/jsconfig.json','jsconfig.json']);
  const script=result.projects.find(item=>item.config==='jsconfig.json');
  assert.deepEqual(script.entries,['script/main.mjs']);assert(script.checkedFiles.includes('shared/text.mjs'));
  assert(!script.checkedFiles.includes('node/jsconfig.json'));
  assert(result.checkedFiles.includes('node/jsconfig.json'));
});

test('检查既有入口与相对依赖，保留源码并且不执行模块初始化器',()=>{
  const root=fixture();const before=readFileSync(join(root,'main.mjs'));
  const result=checkProjectTypes(root);
  assert.equal(result.passed,true,JSON.stringify(result.diagnostics));
  assert(result.checkedFiles.includes('lib/text.mjs'));
  assert(result.checkedFiles.includes('sdk/api.d.ts'));
  assert.equal(result.runtimeExecuted,false);assert.equal(result.filesModified,false);
  assert.equal(result.staticEnvironmentPassed,true);assert.equal(result.staticAnalysisComplete,true);
  assert.deepEqual(result.environmentDiagnostics,[]);assert.deepEqual(result.skippedEntries,[]);
  assert.equal(result.moduleChecks[0].entry,'main.mjs');
  assert.equal(result.entryChecks[0].mainExport,'local_callable_declaration');
  assert.equal(result.runtimeCapabilitiesChecked,false);assert.equal(result.runtimeVerified,false);
  assert.deepEqual(readFileSync(join(root,'main.mjs')),before);
});

test('平铺SDK使用真实Node与Script声明，仍执行strict负例且不运行初始化器',()=>{
  const root=fixture();unlinkSync(join(root,'sdk/api.d.ts'));rmdirSync(join(root,'sdk'));
  for(const name of ['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts']) {
    writeFileSync(join(root,name),readFileSync(join(repo,'crates/omnitrigger-actions/runtime',name)));
  }
  writeFileSync(join(root,'ambient.d.ts'),'/// <reference path="./script-types.d.ts" />\n');
  writeFileSync(join(root,'jsconfig.json'),JSON.stringify({compilerOptions:{strict:true,skipLibCheck:false,target:'ES2020',module:'ESNext',moduleResolution:'Bundler',lib:['ES2020'],types:[]},files:['main.mjs','ambient.d.ts','api.d.ts']}));
  const file=join(root,'main.mjs');
  writeFileSync(file,'/// <reference path="./ambient.d.ts" />\nimport {clean} from "./lib/text.mjs";\n/** @type {ScriptMain} */\nexport function main(ctx) { const result=ctx.clipboard.history.list(); return {count:result.items.length,value:clean(" synthetic ")}; }\n');
  const before=readFileSync(file);const result=checkProjectTypes(root);
  assert.equal(result.passed,true,JSON.stringify(result.diagnostics));
  for(const name of ['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts','ambient.d.ts'])assert(result.checkedFiles.includes(name));
  assert(!result.checkedFiles.includes('sdk/api.d.ts'));
  assert.equal(result.runtimeExecuted,false);assert.equal(result.filesModified,false);
  assert.deepEqual(readFileSync(file),before);
  writeFileSync(file,'/// <reference path="./ambient.d.ts" />\n/** @type {ScriptMain} */\nexport function main(ctx) { return ctx.clipboard.history.get({id:1}); }\n');
  const invalid=checkProjectTypes(root);assert.equal(invalid.passed,false);
  assert(invalid.diagnostics.some(item=>item.file==='main.mjs'&&item.code===2322));
});

test('SDK优先路径损坏时不回退根声明或绕过原拒绝',()=>{
  const root=fixture();
  writeFileSync(join(root,'api.d.ts'),readFileSync(join(repo,'crates/omnitrigger-actions/runtime/api.d.ts')));
  writeFileSync(join(root,'sdk/api.d.ts'),Buffer.from([0xff]));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_utf8_invalid'&&error.location?.file==='sdk/api.d.ts');
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

test('配置JSON语法错误保留工程内文件和一基行列',()=>{
  const root=fixture();const path=join(root,'jsconfig.json');
  const text='{\n  "compilerOptions": {\n    "checkJs": }\n}\n';
  writeFileSync(path,text);
  assert.throws(()=>checkProjectTypes(root),error=>{
    assert.equal(error.code,'project_config_invalid');
    assert.deepEqual(error.location,{file:'jsconfig.json',line:3,column:16});return true;
  });
  assert.equal(readFileSync(path,'utf8'),text);
});

test('CLI配置语法错误输出可定位JSON而不执行入口',()=>{
  const root=fixture();writeFileSync(join(root,'jsconfig.json'),'{\n  "compilerOptions": {\n    "checkJs": }\n}\n');
  const result=spawnSync(process.execPath,[join(repo,'tools/script-sdk-check/check-project-types.mjs'),root],
    {encoding:'utf8',windowsHide:true,timeout:30000,maxBuffer:1024*1024});
  assert.ifError(result.error);assert.equal(result.status,1);assert.equal(result.stdout,'');
  const output=JSON.parse(result.stderr);
  assert.equal(output.passed,false);assert.equal(output.error.code,'project_config_invalid');
  assert.deepEqual(output.error.location,{file:'jsconfig.json',line:3,column:16});
  assert.equal(output.runtimeExecuted,false);assert.equal(output.filesModified,false);
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
  assert.throws(()=>checkProjectTypes(root),error=>{
    assert.equal(error.code,'project_utf8_invalid');assert.deepEqual(error.location,{file:'main.mjs'});return true;
  });
  writeFileSync(join(root,'main.mjs'),'export function main() { return null; }');
  linkSync(join(root,'main.mjs'),join(root,'same-file.mjs'));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_link_path');
});

test('CLI非法UTF8依赖定位到具体包内模块，不回显字节或猜测行列',()=>{
  const root=fixture();writeFileSync(join(root,'lib/text.mjs'),Buffer.from([0xff,0xfe,0x61]));
  const result=spawnSync(process.execPath,[join(repo,'tools/script-sdk-check/check-project-types.mjs'),root],
    {encoding:'utf8',windowsHide:true,timeout:30000,maxBuffer:1024*1024});
  assert.ifError(result.error);assert.equal(result.status,1);assert.equal(result.stdout,'');
  const output=JSON.parse(result.stderr);
  assert.equal(output.error.code,'project_utf8_invalid');
  assert.deepEqual(output.error.location,{file:'lib/text.mjs'});
  assert.equal(output.runtimeExecuted,false);assert.equal(output.filesModified,false);
  assert.deepEqual(readFileSync(join(root,'lib/text.mjs')),Buffer.from([0xff,0xfe,0x61]));
});

test('CLI缺参数返回结构化失败和退出码，不尝试寻找工程',()=>{
  const result=spawnSync(process.execPath,[join(repo,'tools/script-sdk-check/check-project-types.mjs')],
    {encoding:'utf8',windowsHide:true,timeout:10000});
  assert.ifError(result.error);assert.equal(result.status,1);
  const output=JSON.parse(result.stderr);
  assert.equal(output.error.code,'project_directory_required');
  assert.equal(output.runtimeExecuted,false);assert.equal(output.filesModified,false);
});

test('类型通过但没有main具名导出时，统一命令返回环境诊断',()=>{
  const root=fixture();
  writeFileSync(join(root,'main.mjs'),'export function other(){return null;}');
  const result=checkProjectTypes(root);
  assert.deepEqual(result.diagnostics,[]);
  assert.equal(result.passed,false);assert.equal(result.staticEnvironmentPassed,false);
  assert(result.environmentDiagnostics.some(d=>d.code==='entry_main_not_exported'&&d.category==='entry'));
  assert.equal(result.runtimeExecuted,false);
});

test('Bundler能解析的省略扩展名导入被轻量环境检查拒绝',()=>{
  const root=fixture();
  writeFileSync(join(root,'lib/value.js'),'export const value=7;');
  writeFileSync(join(root,'main.mjs'),'import {value} from "./lib/value";export function main(){return value;}');
  const result=checkProjectTypes(root);
  assert.deepEqual(result.diagnostics,[]);
  assert.equal(result.passed,false);
  assert(result.environmentDiagnostics.some(d=>d.code==='module_specifier'&&d.category==='modules'&&d.line===1));
});

test('TypeScript重定向到ts声明时仍按明确js路径检查实际静态依赖',()=>{
  const root=fixture();
  writeFileSync(join(root,'lib/value.ts'),'export const value=7;');
  writeFileSync(join(root,'lib/value.js'),'import "./child.js";export const value=7;');
  writeFileSync(join(root,'lib/child.js'),'throw Error("RUNTIME JS MUST NOT EXECUTE");export const child=true;');
  writeFileSync(join(root,'main.mjs'),'import {value} from "./lib/value.js";export function main(){return value;}');
  const result=checkProjectTypes(root);
  assert.deepEqual(result.diagnostics,[]);assert.equal(result.passed,true);
  assert(result.checkedFiles.includes('lib/value.ts'));
  assert(result.checkedFiles.includes('lib/value.js'));assert(result.checkedFiles.includes('lib/child.js'));
  assert.deepEqual(result.moduleChecks[0].modules,['main.mjs','lib/value.js','lib/child.js']);
  assert.equal(result.runtimeExecuted,false);
});

test('动态目标和再导出保留不确定性，不把警告改成运行成功',()=>{
  const root=fixture();
  writeFileSync(join(root,'main.mjs'),'export function main(){if(false)import(String("not-evaluated"));return null;}');
  const dynamic=checkProjectTypes(root);
  assert.equal(dynamic.passed,true);assert.equal(dynamic.staticAnalysisComplete,false);
  assert(dynamic.environmentDiagnostics.some(d=>d.code==='dynamic_import_unresolved'&&d.severity==='warning'));
  writeFileSync(join(root,'lib/value.js'),'export function run(){return null;}');
  writeFileSync(join(root,'main.mjs'),'export {run as main} from "./lib/value.js";');
  const reexport=checkProjectTypes(root);
  assert.equal(reexport.passed,true);assert.equal(reexport.staticAnalysisComplete,false);
  assert(reexport.environmentDiagnostics.some(d=>d.code==='entry_main_value_unresolved'));
  assert.equal(reexport.runtimeVerified,false);
});

test('同包多个旧js入口的main和局部变量按文件隔离',()=>{
  const root=fixture();
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[
    {runtime:'script',entry:'first.js'},{runtime:'script',entry:'second.js'},
    {runtime:'script',entry:'main.mjs'},
  ]}));
  for(const file of ['first.js','second.js'])writeFileSync(join(root,file),
    '/** @type {ScriptMain} */\nfunction main(ctx){return ctx.input.text;}\nconst localValue=1;');
  const result=checkProjectTypes(root);
  assert.equal(result.passed,true,JSON.stringify(result.diagnostics));
  assert.deepEqual(result.diagnostics,[]);
  assert.deepEqual(result.skippedEntries,[]);
  assert.equal(result.moduleChecks.length,3);assert.equal(result.entryChecks.length,3);
  assert.equal(result.runtimeExecuted,false);assert.equal(result.staticAnalysisComplete,true);
});

test('另一个旧js入口的局部变量不能替当前入口掩盖未定义名称',()=>{
  const root=fixture();
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[
    {runtime:'script',entry:'provider.js'},{runtime:'script',entry:'consumer.js'},
  ]}));
  writeFileSync(join(root,'provider.js'),'const otherEntryValue=1;\n/** @type {ScriptMain} */\nvar main=function(ctx){return ctx.input.text;};');
  writeFileSync(join(root,'consumer.js'),'/** @type {ScriptMain} */\nvar main=function(ctx){return otherEntryValue;};');
  const result=checkProjectTypes(root);
  assert.equal(result.passed,false);
  assert(result.diagnostics.some(item=>item.code===2304&&item.file==='consumer.js'));
  assert.equal(result.runtimeExecuted,false);assert.equal(result.entryChecks.length,2);
});

test('旧js入口的静态import或export不能因类型通过而误放行',()=>{
  for(const source of [
    'import {clean} from "./lib/text.mjs";\n/** @type {ScriptMain} */\nfunction main(ctx){return clean("text");}',
    '/** @type {ScriptMain} */\nexport function main(ctx){return {ok:true};}',
  ]) {
    const root=fixture();
    writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[{runtime:'script',entry:'legacy.js'}]}));
    writeFileSync(join(root,'legacy.js'),source);
    const result=checkProjectTypes(root);assert.deepEqual(result.diagnostics,[]);
    assert.equal(result.passed,false);assert.equal(result.staticEnvironmentPassed,false);
    const diagnostic=result.environmentDiagnostics.find(item=>item.code==='legacy_entry_module_syntax');
    assert.equal(diagnostic.file,'legacy.js');assert.equal(diagnostic.column,1);
    assert.equal(diagnostic.line,source.startsWith('import')?1:2);
    assert.equal(result.staticAnalysisComplete,false);assert.equal(result.runtimeExecuted,false);
    assert.equal(result.entryChecks.length,1);
  }
});

test('CLI旧js模块语法错误可定位且mjs导入js辅助模块保持可用',()=>{
  const root=fixture();
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[{runtime:'script',entry:'legacy.js'}]}));
  writeFileSync(join(root,'legacy.js'),'/** @type {ScriptMain} */\nexport function main(ctx){return null;}');
  const result=spawnSync(process.execPath,[join(repo,'tools/script-sdk-check/check-project-types.mjs'),root],
    {encoding:'utf8',windowsHide:true,timeout:30000,maxBuffer:1024*1024});
  assert.ifError(result.error);assert.equal(result.status,1);assert.equal(result.stderr,'');
  const output=JSON.parse(result.stdout);assert.equal(output.passed,false);
  assert(output.environmentDiagnostics.some(item=>item.code==='legacy_entry_module_syntax'&&item.file==='legacy.js'&&item.line===2));
  assert.equal(output.runtimeExecuted,false);assert.equal(output.filesModified,false);
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[{runtime:'script',entry:'main.mjs'}]}));
  writeFileSync(join(root,'lib/value.js'),'export const value=1;');
  writeFileSync(join(root,'main.mjs'),'import {value} from "./lib/value.js";export function main(){return value;}');
  const module=checkProjectTypes(root);assert.equal(module.passed,true,JSON.stringify(module));
});

test('旧js函数体入口继续做类型检查，不强加ESM导出要求',()=>{
  const root=fixture();
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[{runtime:'script',entry:'legacy.js'}]}));
  writeFileSync(join(root,'legacy.js'),'function main(){return null;}');
  const result=checkProjectTypes(root);
  assert.equal(result.passed,true);assert.equal(result.moduleChecks.length,1);assert.equal(result.entryChecks.length,1);
  assert.deepEqual(result.skippedEntries,[]);
  assert(result.environmentDiagnostics.some(d=>d.code==='legacy_entry_main_signature_unresolved'));
  assert.equal(result.staticAnalysisComplete,false);
  assert(!result.checkedFiles.includes('main.mjs'));
});

test('旧js唯一同步main被完整分析，动态求值或重绑定保留不完整且不执行',()=>{
  const root=fixture();
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[{runtime:'script',entry:'legacy.js'}]}));
  const file=join(root,'legacy.js');
  const source='/** @type {ScriptMain} */\nfunction main(ctx){return {value:ctx.input.value};}';
  writeFileSync(file,source);
  const result=checkProjectTypes(root);
  assert.equal(result.passed,true,JSON.stringify(result));assert.equal(result.staticAnalysisComplete,true);
  assert.deepEqual(result.skippedEntries,[]);assert.equal(result.entryChecks[0].entryProtocol,'legacy_function_body');
  assert.equal(result.entryChecks[0].mainExport,'legacy_callable_declaration');
  assert.equal(result.moduleChecks[0].staticGraphComplete,true);assert.deepEqual(result.moduleChecks[0].modules,['legacy.js']);
  assert.deepEqual(result.moduleChecks[0].edges,[]);assert.equal(result.runtimeVerified,false);assert.equal(result.runtimeExecuted,false);
  assert.equal(readFileSync(file,'utf8'),source);
  writeFileSync(file,'// @ts-nocheck\nfunction main(ctx){eval(ctx.input.code);return null;}');
  const dynamic=checkProjectTypes(root);assert.equal(dynamic.passed,true);assert.equal(dynamic.staticAnalysisComplete,false);
  assert.equal(dynamic.moduleChecks[0].staticGraphComplete,false);
  assert(dynamic.environmentDiagnostics.some(d=>d.code==='legacy_entry_dynamic_code'&&d.file==='legacy.js'&&d.line===2));
  writeFileSync(file,'// @ts-nocheck\nfunction main(ctx){return null;} main=ctx.input.entry;');
  const rebound=checkProjectTypes(root);assert.equal(rebound.passed,true);assert.equal(rebound.staticAnalysisComplete,false);
  assert(rebound.environmentDiagnostics.some(d=>d.code==='legacy_entry_main_reassigned'));
});

test('旧js生成器及不存在main不会因ts-nocheck被标记完整',()=>{
  const root=fixture();
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[{runtime:'script',entry:'legacy.js'}]}));
  for(const [source,code] of [
    ['function* main(ctx){yield 1;}','entry_main_generator'],
    ['function other(ctx){return null;}','legacy_entry_main_missing'],
  ]) {
    writeFileSync(join(root,'legacy.js'),'// @ts-nocheck\n'+source);
    const result=checkProjectTypes(root);assert.equal(result.passed,false);assert.equal(result.staticAnalysisComplete,false);
    assert(result.environmentDiagnostics.some(d=>d.code===code),JSON.stringify(result));assert.equal(result.runtimeExecuted,false);
  }
});

test('类型预算内超过轻量宿主1MiB的模块图仍拒绝',()=>{
  const root=fixture();
  const imports=[];
  for(let i=0;i<9;i++) {
    imports.push('import "./lib/m'+i+'.js";');
    writeFileSync(join(root,'lib/m'+i+'.js'),'export const value=1;//'+'x'.repeat(120*1024));
  }
  writeFileSync(join(root,'main.mjs'),imports.join('\n')+'\nexport function main(){return null;}');
  const result=checkProjectTypes(root);
  assert.deepEqual(result.diagnostics,[]);
  assert.equal(result.passed,false);assert.equal(result.staticAnalysisComplete,false);
  assert(result.environmentDiagnostics.some(d=>d.code==='module_total_limit'));
});

test('补读实际js边仍先检查硬链接属性，不绕过已有元数据防护',()=>{
  const root=fixture();
  writeFileSync(join(root,'lib/value.ts'),'export const value=7;');
  writeFileSync(join(root,'lib/value.js'),'export const value=7;');
  linkSync(join(root,'lib/value.js'),join(root,'lib/linked.js'));
  writeFileSync(join(root,'main.mjs'),'import {value} from "./lib/value.js";export function main(){return value;}');
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_link_path');
});

test('统一CLI对环境失败输出JSON和退出码1，不执行作者初始化器',()=>{
  const root=fixture();
  writeFileSync(join(root,'main.mjs'),'throw Error("AUTHOR MUST NOT EXECUTE");export function other(){return null;}');
  const result=spawnSync(process.execPath,[join(repo,'tools/script-sdk-check/check-project-types.mjs'),root],
    {encoding:'utf8',windowsHide:true,timeout:30000,maxBuffer:1024*1024});
  assert.ifError(result.error);assert.equal(result.status,1,result.stderr);
  const output=JSON.parse(result.stdout);
  assert.deepEqual(output.diagnostics,[]);
  assert(output.environmentDiagnostics.some(d=>d.code==='entry_main_not_exported'));
  assert.equal(output.runtimeExecuted,false);assert.equal(output.filesModified,false);
});


test('模块入口和辅助模块都必须显式接收上下文',()=>{
  for(const [file,source] of [
    ['main.mjs','ctx.delay(0);export function main(){return null;}'],
    ['lib/text.mjs','/** @param {string} text */\nexport function clean(text){return ctx.input.text;}'],
  ]) {
    const root=fixture();writeFileSync(join(root,file),source);
    const result=checkProjectTypes(root);
    assert.equal(result.passed,false);
    assert(result.diagnostics.some(d=>d.code===2304),JSON.stringify(result.diagnostics));
    assert.equal(result.runtimeExecuted,false);assert.equal(result.filesModified,false);
  }
});


function dualSdkFixture() {
  const root=fixture();
  writeFileSync(join(root,'sdk/api.d.ts'),'export interface NodeActionContext { input: Record<string,unknown>; }\n');
  writeFileSync(join(root,'sdk/script-types.d.ts'),sdk);
  writeFileSync(join(root,'main.mjs'),'throw Error("DECLARATION CHECK MUST NOT EXECUTE");\n/** @type {ScriptMain} */\nexport function main(ctx) { return {text:String(ctx.input.text ?? "")}; }\n');
  writeFileSync(join(root,'unused-node.mjs'),'THIS FILE IS NOT A SCRIPT ENTRY AND MUST NOT BE READ');
  writeFileSync(join(root,'jsconfig.json'),JSON.stringify({compilerOptions:{strict:true,target:'ES2020',module:'ESNext',moduleResolution:'Bundler',lib:['ES2020']},files:['main.mjs','unused-node.mjs','sdk/script-types.d.ts']}));
  return root;
}

test('files列明的同步声明进入混合SDK检查，但不扩大到未使用Node源码',()=>{
  const root=dualSdkFixture();const entry=readFileSync(join(root,'main.mjs'));
  const result=checkProjectTypes(root);
  assert.equal(result.passed,true,JSON.stringify(result.diagnostics));
  assert(result.checkedFiles.includes('sdk/script-types.d.ts'));
  assert(!result.checkedFiles.includes('unused-node.mjs'));
  assert.equal(result.runtimeExecuted,false);assert.equal(result.filesModified,false);
  assert.deepEqual(readFileSync(join(root,'main.mjs')),entry);
});

test('显式声明中的类型错误保留实际文件和行列',()=>{
  const root=dualSdkFixture();
  writeFileSync(join(root,'sdk/broken.d.ts'),'declare const impossible: MissingConfiguredType;\n');
  const path=join(root,'jsconfig.json');const config=JSON.parse(readFileSync(path,'utf8'));
  config.files.push('sdk/broken.d.ts');writeFileSync(path,JSON.stringify(config));
  const result=checkProjectTypes(root);
  assert.equal(result.passed,false);
  assert(result.diagnostics.some(d=>d.code===2304&&d.file==='sdk/broken.d.ts'&&d.line===1&&d.column===27));
  assert.equal(result.runtimeExecuted,false);
});

test('files不允许隐藏清单中的生成器入口约束',()=>{
  const root=dualSdkFixture();const path=join(root,'jsconfig.json');
  const config=JSON.parse(readFileSync(path,'utf8'));config.files=['sdk/script-types.d.ts'];
  writeFileSync(path,JSON.stringify(config));
  writeFileSync(join(root,'main.mjs'),'/** @type {ScriptMain} */\nexport function* main(ctx){yield {text:String(ctx.input.text)};}');
  const result=checkProjectTypes(root);
  assert.equal(result.passed,false);
  assert(result.environmentDiagnostics.some(d=>d.code==='entry_main_generator'));
  assert.deepEqual(result.entries,['main.mjs']);
});

test('缺失的显式声明是类型诊断，不静默略过',()=>{
  const root=dualSdkFixture();const path=join(root,'jsconfig.json');
  const config=JSON.parse(readFileSync(path,'utf8'));config.files.push('sdk/missing.d.ts');
  writeFileSync(path,JSON.stringify(config));
  const result=checkProjectTypes(root);
  assert.equal(result.passed,false);assert(result.diagnostics.some(d=>d.code===6053));
  assert(!result.checkedFiles.includes('sdk/missing.d.ts'));
});

test('显式声明拒绝越界和通配符，错误定位到工程配置',()=>{
  for(const name of ['../outside.d.ts','/outside.d.ts','C:/outside.d.ts','sdk/*.d.ts']) {
    const root=dualSdkFixture();const path=join(root,'jsconfig.json');
    const config=JSON.parse(readFileSync(path,'utf8'));config.files.push(name);
    writeFileSync(path,JSON.stringify(config));
    assert.throws(()=>checkProjectTypes(root),error=>{
      assert.equal(error.code,'project_config_unsupported');
      assert.deepEqual(error.location,{file:'jsconfig.json'});return true;
    });
  }
});

test('files数组类型与数量有界，不让无效路径进入读取',()=>{
  for(const files of [null,'sdk/api.d.ts',[null],Array(257).fill('sdk/api.d.ts')]) {
    const root=fixture();writeFileSync(join(root,'jsconfig.json'),JSON.stringify({files}));
    assert.throws(()=>checkProjectTypes(root),error=>{
      assert.equal(error.code,'project_config_invalid');
      assert.deepEqual(error.location,{file:'jsconfig.json'});return true;
    });
  }
});

test('配置声明继续遵守硬链接和容量防护',()=>{
  const root=dualSdkFixture();const declaration=join(root,'sdk/script-types.d.ts');
  linkSync(declaration,join(root,'sdk/linked.d.ts'));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_link_path');
  unlinkSync(join(root,'sdk/linked.d.ts'));
  writeFileSync(declaration,'//'+ 'x'.repeat(512*1024));
  assert.throws(()=>checkProjectTypes(root),error=>error.code==='project_file_limit');
});

test('同一显式声明去重，SDK快照不会因重复配置产生伪冲突',()=>{
  const root=dualSdkFixture();const path=join(root,'jsconfig.json');
  const config=JSON.parse(readFileSync(path,'utf8'));config.files.push('sdk/script-types.d.ts','sdk/api.d.ts');
  writeFileSync(path,JSON.stringify(config));
  const result=checkProjectTypes(root);
  assert.equal(result.passed,true,JSON.stringify(result.diagnostics));
  assert.equal(result.checkedFiles.filter(name=>name==='sdk/script-types.d.ts').length,1);
});


test('编译选项定位：合法JSON中的错误值指出原始行列',()=>{
  const root=fixture();
  writeFileSync(join(root,'jsconfig.json'),'{\n  "compilerOptions": {\n    "strict": "yes"\n  },\n  "files": ["main.mjs"]\n}\n');
  assert.throws(()=>checkProjectTypes(root),error=>{
    assert.equal(error.code,'project_config_invalid');
    assert.deepEqual(error.location,{file:'jsconfig.json',line:3,column:15});return true;
  });
});
test('编译选项定位：拼错选项定位键名，不回显配置正文',()=>{
  const root=fixture();
  writeFileSync(join(root,'jsconfig.json'),'{\n  "compilerOptions": {\n    "strcit": true\n  }\n}\n');
  assert.throws(()=>checkProjectTypes(root),error=>{
    assert.equal(error.code,'project_config_invalid');
    assert.deepEqual(error.location,{file:'jsconfig.json',line:3,column:5});
    assert(!error.message.includes('strcit'));return true;
  });
});
test('编译选项定位：CLI保留结构化位置，作者初始化器不执行',()=>{
  const root=fixture();
  writeFileSync(join(root,'jsconfig.json'),'{\n  "compilerOptions": {\n    "strict": "synthetic-private-value"\n  }\n}\n');
  const result=spawnSync(process.execPath,[join(repo,'tools/script-sdk-check/check-project-types.mjs'),root],{encoding:'utf8',windowsHide:true,timeout:30000});
  assert.ifError(result.error);assert.equal(result.status,1);
  const output=JSON.parse(result.stderr);
  assert.equal(output.error.code,'project_config_invalid');
  assert.deepEqual(output.error.location,{file:'jsconfig.json',line:3,column:15});
  assert(!result.stderr.includes('synthetic-private-value'));
  assert.equal(output.runtimeExecuted,false);assert.equal(output.filesModified,false);
});
test('编译选项定位：没有文件发现回调读取，include不扩大范围',()=>{
  const root=fixture();
  writeFileSync(join(root,'jsconfig.json'),'{\n  "include": ["../never-read/**"],\n  "compilerOptions": {\n    "strict": "yes"\n  }\n}\n');
  assert.throws(()=>checkProjectTypes(root),error=>{
    assert.equal(error.code,'project_config_invalid');
    assert.deepEqual(error.location,{file:'jsconfig.json',line:4,column:15});return true;
  });
});
test('编译选项定位：有效生成工程的检查结果和作者代码保持不变',()=>{
  const root=fixture();const before=readFileSync(join(root,'main.mjs'));
  const result=checkProjectTypes(root);
  assert.equal(result.passed,true);assert.equal(result.runtimeExecuted,false);assert.equal(result.filesModified,false);
  assert.deepEqual(readFileSync(join(root,'main.mjs')),before);
});


test('JSON作者环境契约：本地SDK和全部入口覆盖，六类错误仍被拒绝',()=>{
  const packageDirectory=join(repo,'examples/actions/json-text');
  const configPath=join(packageDirectory,'jsconfig.json');
  const loaded=ts.readConfigFile(configPath,ts.sys.readFile);assert.equal(loaded.error,undefined);
  const parsed=ts.parseJsonConfigFileContent(loaded.config,ts.sys,packageDirectory);assert.deepEqual(parsed.errors,[]);
  const root=fixture(),options={...parsed.options,noEmit:true};
  const prefix=relative(root,packageDirectory).replaceAll('\\','/');
  function typeFixture(name) {
    const path=join(root,name);
    const source=readFileSync(join(repo,'tools/script-sdk-check/fixtures',name),'utf8');
    writeFileSync(path,source.replaceAll('./package/',prefix+'/'));return path;
  }
  const positive=typeFixture('json-author-positive.mjs');
  const program=ts.createProgram([...parsed.fileNames,positive],options);
  assert.deepEqual(ts.getPreEmitDiagnostics(program),[]);
  const files=new Set(program.getSourceFiles().map(file=>resolve(file.fileName)));
  const inside=(parent,path)=>{const name=relative(parent,path);return name===''||!isAbsolute(name)&&name!=='..'&&!name.startsWith('../')&&!name.startsWith('..\\');};
  const library=dirname(ts.getDefaultLibFilePath(options));
  for(const path of files)assert(path===positive||inside(packageDirectory,path)||inside(library,path),'JSON作者声明不得回退到包外仓库源码');
  const manifest=JSON.parse(readFileSync(join(packageDirectory,'action.json'),'utf8'));
  for(const action of manifest.actions) {
    const entry=resolve(packageDirectory,action.entry);assert(inside(packageDirectory,entry));assert(files.has(entry),'动作入口必须参与实际strict检查');
  }
  for(const name of ['json-types.d.ts','sdk/api.d.ts','sdk/script-types.d.ts','sdk/clipboard-history-api.d.ts'])assert(files.has(join(packageDirectory,name)),'本地SDK声明必须参与实际检查');
  const negative=typeFixture('json-author-negative.mjs');
  const diagnostics=ts.getPreEmitDiagnostics(ts.createProgram([...parsed.fileNames,negative],options));
  const expected=[[3,2322],[5,2322],[7,2322],[9,2322],[11,18046],[13,18046]];
  assert.equal(diagnostics.length,expected.length);
  diagnostics.forEach((item,index)=>{
    assert(item.file&&typeof item.start==='number');assert.equal(resolve(item.file.fileName),negative);
    const position=item.file.getLineAndCharacterOfPosition(item.start);assert.deepEqual([position.line+1,item.code],expected[index]);
  });
});


test('导入属性在完整工程检查中返回准确行列，不执行初始化器或改写源码',()=>{
  const root=fixture();
  const text="// owned fixture\nimport {clean} from './lib/text.mjs' with { mode:'unsupported' };\n/** @type {ScriptMain} */\nexport function main(ctx) { return {text:clean(' owned ')}; }\n";
  const file=join(root,'main.mjs');writeFileSync(file,text);
  const result=checkProjectTypes(root);
  assert.equal(result.passed,false);
  assert.deepEqual(result.diagnostics,[]);
  assert.equal(result.staticEnvironmentPassed,false);
  const item=result.environmentDiagnostics.find(d=>d.code==='module_import_attributes');
  assert(item);assert.equal(item.file,'main.mjs');assert.equal(item.line,2);
  assert.equal(item.column,text.split('\n')[1].indexOf('with')+1);
  assert.equal(result.runtimeExecuted,false);assert.equal(result.filesModified,false);
  assert.equal(readFileSync(file,'utf8'),text);
});

test('轻量工程async/Promise入口与TextEncoder严格通过，静态检查不执行作者代码',()=>{
  const root=dualSdkFixture();
  writeFileSync(join(root,'main.mjs'),'/** @type {ScriptAsyncMain} */\nexport async function main(ctx){return {bytes:Array.from(new TextEncoder().encode(String(ctx.input.text)))};}');
  const result=checkProjectTypes(root);
  assert.equal(result.passed,true,JSON.stringify(result));assert.equal(result.staticAnalysisComplete,true);
  assert.equal(result.entryChecks[0].mainExport,'local_callable_declaration');assert.equal(result.runtimeExecuted,false);
});

test('Script default-only即使类型正确也不能认定为可运行入口',()=>{
  const root=dualSdkFixture();
  writeFileSync(join(root,'main.mjs'),'/** @type {ScriptAsyncMain} */\nexport default async function main(ctx){return {bytes:Array.from(new TextEncoder().encode(String(ctx.input.text)))};}');
  const result=checkProjectTypes(root);
  assert.deepEqual(result.diagnostics,[]);assert.equal(result.passed,false);assert.equal(result.staticAnalysisComplete,false);
  assert.equal(result.entryChecks[0].mainExport,'missing_named_export');
  assert(result.environmentDiagnostics.some(d=>d.code==='entry_main_not_exported'));
  assert.equal(result.runtimeExecuted,false);assert.equal(result.runtimeVerified,false);
});
