import assert from 'node:assert/strict';
import test from 'node:test';
import {analyzeScriptModules} from './analyze-script-modules.mjs';

const snapshot=(text,extra=[])=>({entry:'main.mjs',sources:[{path:'main.mjs',text},...extra]});
const codes=result=>result.diagnostics.map(d=>d.code);
const source=(path,text='export const value=1;')=>({path,text});

test('静态导入和再导出得到确定图，源码初始化器不会执行',()=>{
  const input=snapshot('globalThis.__moduleAuditExecuted=true;throw Error("MUST NOT RUN");\nimport {value} from "./lib/value.js";\nexport * from "./lib/reexport.mjs";',
    [source('lib/value.js'),source('lib/reexport.mjs','export {value} from "./value.js";')]);
  const before=JSON.stringify(input);globalThis.__moduleAuditExecuted=false;
  const result=analyzeScriptModules(input);
  assert.equal(result.passed,true);assert.equal(result.staticGraphComplete,true);
  assert.deepEqual(result.modules,['main.mjs','lib/value.js','lib/reexport.mjs']);
  assert.equal(result.edges.length,3);assert.equal(result.runtimeExecuted,false);
  assert.equal(result.filesRead,false);assert.equal(result.filesModified,false);
  assert.equal(globalThis.__moduleAuditExecuted,false);delete globalThis.__moduleAuditExecuted;
  assert.equal(JSON.stringify(input),before);
});
test('注释和普通字符串不是导入',()=>{
  const result=analyzeScriptModules(snapshot('// import x from "node:fs";\nconst x="import(\'./missing.js\')";export function main(){return x;}'));
  assert.equal(result.passed,true);assert.deepEqual(result.edges,[]);
});
test('省略扩展名、包名、Node内置和非JS扩展被明确诊断',()=>{
  for(const name of ['./value','package','node:fs','./value.json','./value.ts','./VALUE.JS']) {
    const result=analyzeScriptModules(snapshot('import "'+name+'";'));
    assert(codes(result).includes('module_specifier'),name);
    assert.equal(result.passed,false);
    assert.equal(result.diagnostics[0].file,'main.mjs');
    assert.equal(result.diagnostics[0].line,1);
    assert.equal(result.diagnostics[0].column,8);
  }
});
test('包内父目录引用可用，越界导入失败，循环只计一次',()=>{
  const ok=analyzeScriptModules(snapshot('import "./lib/a.js";',[
    source('lib/a.js','import "../main.mjs";export const x=1;')]));
  assert.equal(ok.passed,true);assert.equal(ok.modules.length,2);
  assert(codes(analyzeScriptModules(snapshot('import "../outside.js";'))).includes('module_path'));
});
test('Windows设备名、非法片段、URL、查询和反斜杠不补全',()=>{
  for(const path of ['./CON.js','./LPT¹.mjs','./name./value.js','./name /value.js','./a//b.js',
    './x.js?q=1','./x.js#part','./%78.js','file:///x.js','C:/x.js','./x\\y.js','./a|b.js']) {
    const result=analyzeScriptModules(snapshot('import '+JSON.stringify(path)+';'));
    assert.equal(result.passed,false,path);assert(codes(result).includes('module_specifier'),path);
  }
});
test('缺失文件仅表示提供的快照不完整，不尝试读取磁盘',()=>{
  const result=analyzeScriptModules(snapshot('import "./not-provided.js";'));
  assert.deepEqual(codes(result),['snapshot_module_missing']);assert.equal(result.filesRead,false);
});
test('64个静态模块通过，第65个拒绝',()=>{
  const make=count=>snapshot(Array.from({length:count-1},(_,i)=>'import "./m'+i+'.js";').join('\n'),
    Array.from({length:count-1},(_,i)=>source('m'+i+'.js')));
  assert.equal(analyzeScriptModules(make(64)).passed,true);
  const result=analyzeScriptModules(make(65));
  assert.equal(result.modules.length,64);assert(codes(result).includes('module_total_limit'));
});
test('字节预算按UTF8计算，128KiB边界可用',()=>{
  assert.equal(analyzeScriptModules(snapshot('//'+ 'x'.repeat(128*1024-2))).passed,true);
  assert(codes(analyzeScriptModules(snapshot('//'+ 'x'.repeat(128*1024-1)))).includes('module_file_limit'));
  assert(codes(analyzeScriptModules(snapshot('//'+ '中'.repeat(44000)))).includes('module_file_limit'));
});
test('静态源码合计1MiB通过，再加一个模块明确失败',()=>{
  const make=count=>{
    const imports=Array.from({length:count-1},(_,i)=>'import "./m'+i+'.js";').join('\n')+'\n//';
    return snapshot(imports+'x'.repeat(128*1024-imports.length),
      Array.from({length:count-1},(_,i)=>source('m'+i+'.js','//'+'x'.repeat(128*1024-2))));
  };
  const ok=analyzeScriptModules(make(8));assert.equal(ok.passed,true);assert.equal(ok.totalBytes,1024*1024);
  assert(codes(analyzeScriptModules(make(9))).includes('module_total_limit'));
});
test('非字面动态导入不执行表达式，明确保留未分析范围',()=>{
  const result=analyzeScriptModules(snapshot('export function main(){return import(globalThis.__MUST_NOT_CALL());}'));
  assert.equal(result.passed,true);assert.equal(result.staticGraphComplete,false);
  assert.deepEqual(codes(result),['dynamic_import_unresolved']);
  assert.equal(result.diagnostics[0].severity,'warning');assert.equal(result.runtimeVerified,false);
});
test('字面动态导入只检查路径，预算不假装已覆盖',()=>{
  const input=snapshot('export function main(){return import("./later.js");}',[source('later.js')]);
  const result=analyzeScriptModules(input);
  assert.equal(result.passed,true);assert.equal(result.staticGraphComplete,false);
  assert.deepEqual(result.modules,['main.mjs']);assert.equal(result.edges[0].kind,'dynamic_literal');
  assert(codes(result).includes('dynamic_import_deferred'));
  const missing=analyzeScriptModules(snapshot('import("./later.js");'));
  assert.equal(missing.passed,true);assert(codes(missing).includes('snapshot_module_missing'));
  assert.equal(missing.diagnostics.at(-1).severity,'warning');
  assert.equal(analyzeScriptModules(snapshot('import("node:fs");')).passed,false);
});
test('入口、重复路径、非规范路径与坏Unicode快照明确拒绝',()=>{
  for(const entry of ['main.js','./main.mjs','../main.mjs','.hidden.mjs','/main.mjs']) {
    assert(codes(analyzeScriptModules({...snapshot(''),entry})).includes('module_entry'),entry);
  }
  assert(codes(analyzeScriptModules(snapshot('',[source('main.mjs')]))).includes('snapshot_invalid'));
  assert(codes(analyzeScriptModules(snapshot('',[source('./alias.js')]))).includes('snapshot_invalid'));
  assert(codes(analyzeScriptModules(snapshot('\ud800'))).includes('snapshot_utf8'));
  for(const invalid of [null,{},[],{entry:'main.mjs',sources:[{path:'main.mjs',text:4}]}]) {
    assert(codes(analyzeScriptModules(invalid)).includes('snapshot_invalid'));
  }
});
test('快照限额独立于静态加载预算，未引用文件不会被加载',()=>{
  const unused=Array.from({length:255},(_,i)=>source('unused'+i+'.js'));
  const ok=analyzeScriptModules(snapshot('',unused));assert.equal(ok.passed,true);assert.equal(ok.modules.length,1);
  assert(codes(analyzeScriptModules(snapshot('',[...unused,source('overflow.js')]))).includes('snapshot_limit'));
  const large=source('large.js','x'.repeat(8*1024*1024+1));
  assert(codes(analyzeScriptModules(snapshot('',[large]))).includes('snapshot_limit'));
});
test('坏语法不会假报完整图，复杂度错误保持结构化',()=>{
  const broken=analyzeScriptModules(snapshot('export function main( {'));
  assert(codes(broken).includes('module_syntax_invalid'));
  assert.equal(broken.passed,false);assert.equal(broken.staticGraphComplete,false);
  const deep=analyzeScriptModules(snapshot('const x='+'('.repeat(12000)+'1'+')'.repeat(12000)+';'));
  assert.equal(deep.passed,false);assert(codes(deep).includes('module_analysis_limit'));
});


test('静态import与再导出的属性块明确拒绝并定位，包含Boa不支持的空属性块',()=>{
  const forms=[
    "import { value } from './lib/value.js' with { mode:'strict' };",
    "import './lib/value.js' with { type:'json' };",
    "export { value } from './lib/value.js' with { mode:'strict' };",
    "export * from './lib/value.js' with { mode:'strict' };",
    "import './lib/value.js' assert { type:'json' };",
    "import * as value from './lib/value.js' with { mode:'strict' };",
    "export * as value from './lib/value.js' with { mode:'strict' };",
    "import './lib/value.js' with {};",
    "export { value } from './lib/value.js' with {};",
    "export * from './lib/value.js' assert {};",
  ];
  for(const text of forms) {
    const result=analyzeScriptModules(snapshot(text,[source('lib/value.js')]));
    assert.equal(result.passed,false,text);
    assert.equal(result.staticGraphComplete,false);
    const item=result.diagnostics.find(d=>d.code==='module_import_attributes');
    assert(item,text);assert.equal(item.file,'main.mjs');assert.equal(item.line,1);
    const marker=text.includes(' with ')?'with':'assert';
    assert.equal(item.column,text.indexOf(marker)+1);
    assert.equal(item.severity,'error');
    assert.equal(result.edges.length,0);assert.deepEqual(result.modules,['main.mjs']);
    assert.equal(result.runtimeExecuted,false);assert.equal(result.filesRead,false);assert.equal(result.filesModified,false);
  }
  for(const text of ["import './lib/value.js';","export { value } from './lib/value.js';"]) {
    assert.equal(analyzeScriptModules(snapshot(text,[source('lib/value.js')])).passed,true);
  }
});

test('属性样式的注释字符串和模板文本不会触发诊断',()=>{
  const text=`// import './missing.js' with { type:'json' };
/* export * from './missing.js' assert { type:'json' }; */
const text="import './missing.js' with { mode:'strict' }";
const template=\`export * from './missing.js' with {};\`;
export {text,template};`;
  const result=analyzeScriptModules(snapshot(text));
  assert.equal(result.passed,true);assert.equal(result.staticGraphComplete,true);
  assert.deepEqual(result.diagnostics,[]);assert.deepEqual(result.edges,[]);
});
test('动态import第二参数保留未决警告，不把options表达式当静态导入属性或执行它',()=>{
  const options=['{}','{with:{}}',"{custom:'owned'}","{with:{mode:'strict'}}",
    "{assert:{type:'json'}}",'undefined','null','4','options',
    '{get with(){throw Error("GETTER MUST NOT RUN");}}','globalThis.__MUST_NOT_CALL()'];
  for(const option of options) {
    const input=snapshot(`export function main(){return import('./later.js', ${option});}`,[source('later.js')]);
    const before=JSON.stringify(input);const result=analyzeScriptModules(input);
    assert.equal(result.passed,true,option);assert.equal(result.staticGraphComplete,false,option);
    assert.deepEqual(codes(result),['dynamic_import_unresolved'],option);
    assert.equal(result.diagnostics[0].severity,'warning');
    assert.deepEqual(result.edges,[]);assert.deepEqual(result.modules,['main.mjs']);
    assert.equal(result.runtimeExecuted,false);assert.equal(result.runtimeVerified,false);
    assert.equal(result.filesRead,false);assert.equal(result.filesModified,false);
    assert.equal(JSON.stringify(input),before);
  }
});

test('JS快照中的TS专有语法明确拒绝并定位，不转译或继续建立导入边',()=>{
  for(const text of [
    'export function main(ctx: any){return null;}',
    'interface Value {a: string}\nexport function main(){return null;}',
    'export function main(){return 1 as number;}',
    'import type {Value} from "./value.js"; export function main(){return null;}',
  ]) {
    const input=snapshot('// 中文定位\n'+text,[source('value.js')]);
    const before=JSON.stringify(input),result=analyzeScriptModules(input);
    assert.equal(result.passed,false,text);assert.equal(result.staticGraphComplete,false);
    assert(result.diagnostics.length>0,text);
    for(const item of result.diagnostics) {
      assert.equal(item.code,'module_typescript_syntax');assert.equal(item.severity,'error');
      assert.equal(item.file,'main.mjs');assert.equal(item.line,2);assert(item.column>=1);
    }
    assert.deepEqual(result.edges,[]);assert.deepEqual(result.modules,['main.mjs']);
    assert.equal(result.runtimeExecuted,false);assert.equal(result.filesRead,false);assert.equal(result.filesModified,false);
    assert.equal(JSON.stringify(input),before);
  }
});
test('相对JS依赖中的TS语法也拒绝，未被静态加载的源码不扩大检查范围',()=>{
  const bad=source('lib/value.js','export const value: number = 1;');
  const result=analyzeScriptModules(snapshot('import {value} from "./lib/value.js";export function main(){return value;}',[bad]));
  assert.equal(result.passed,false);assert.equal(result.staticGraphComplete,false);
  assert.equal(result.diagnostics[0].code,'module_typescript_syntax');assert.equal(result.diagnostics[0].file,'lib/value.js');
  assert.equal(analyzeScriptModules(snapshot('export function main(){return null;}',[bad])).passed,true);
});
test('合法JSDoc、相对模块和default async仍按原有静态语义通过',()=>{
  const input=snapshot(`import {value} from './lib/value.js';
/** @param {{input: {name: string}}} ctx */
export function main(ctx){return {name:ctx.input.name,value};}
export default async function helper(){return null;}`,[source('lib/value.js','/** @type {number} */\nexport const value=1;')]);
  const result=analyzeScriptModules(input);
  assert.equal(result.passed,true);assert.equal(result.staticGraphComplete,true);
  assert.deepEqual(result.diagnostics,[]);assert.deepEqual(result.modules,['main.mjs','lib/value.js']);
  assert.equal(analyzeScriptModules(snapshot('export default async function main(){return null;}')).passed,true);
});
