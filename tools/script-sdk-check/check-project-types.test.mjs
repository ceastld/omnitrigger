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
  assert.deepEqual(result.skippedEntries.map(item=>item.entry),['first.js','second.js']);
  assert.equal(result.moduleChecks.length,1);assert.equal(result.entryChecks.length,1);
  assert.equal(result.runtimeExecuted,false);assert.equal(result.staticAnalysisComplete,false);
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
  assert.equal(result.runtimeExecuted,false);assert.deepEqual(result.entryChecks,[]);
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
    assert.deepEqual(result.entryChecks,[]);
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
  assert.equal(result.passed,true);assert.deepEqual(result.moduleChecks,[]);assert.deepEqual(result.entryChecks,[]);
  assert.deepEqual(result.skippedEntries,[{entry:'legacy.js',reason:'legacy_function_body_protocol'}]);
  assert.equal(result.staticAnalysisComplete,false);
  assert(!result.checkedFiles.includes('main.mjs'));
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

test('files不允许隐藏清单中的同步入口约束',()=>{
  const root=dualSdkFixture();const path=join(root,'jsconfig.json');
  const config=JSON.parse(readFileSync(path,'utf8'));config.files=['sdk/script-types.d.ts'];
  writeFileSync(path,JSON.stringify(config));
  writeFileSync(join(root,'main.mjs'),'/** @type {ScriptMain} */\nexport async function main(ctx){return {text:String(ctx.input.text)};}');
  const result=checkProjectTypes(root);
  assert.equal(result.passed,false);
  assert(result.environmentDiagnostics.some(d=>d.code==='entry_main_async'));
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
