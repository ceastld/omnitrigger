import assert from 'node:assert/strict';
import test from 'node:test';
import {analyzeScriptEntry} from './analyze-script-entry.mjs';
import {analyzeScriptModules} from './analyze-script-modules.mjs';
const inspect=text=>analyzeScriptEntry({file:'nested/work.js',text});
const graph=text=>analyzeScriptModules({protocol:'legacy_function_body',entry:'nested/work.js',sources:[{path:'nested/work.js',text}]});
const codes=r=>r.diagnostics.map(d=>d.code);
test('旧协议单一同步main与安全常量完整，参数名不硬编码',()=>{
 for(const text of ['function main(ctx){return ctx.input;}',
  "'use strict';const count=1;function main(context){return {value:context.input,count};}",
  'const main=ctx=>ctx.input;', 'var main=function(ctx){return ctx.input;};']) {
  const input={file:'nested/work.js',text},copy=JSON.stringify(input),result=analyzeScriptEntry(input);
  assert.equal(result.passed,true,JSON.stringify(result));assert.equal(result.mainExport,'legacy_callable_declaration');
  assert.equal(result.staticAnalysisComplete,true);assert.equal(result.entryProtocol,'legacy_function_body');
  assert.equal(result.runtimeVerified,false);assert.equal(result.runtimeExecuted,false);assert.equal(result.filesRead,false);assert.equal(result.filesModified,false);
  assert.equal(JSON.stringify(input),copy);assert.equal(graph(text).staticGraphComplete,true);
 }
});
test('缺失、重声明、非函数、async和生成器有准确拒绝与定位',()=>{
 for(const [text,code] of [
  ['// marker\nfunction other(ctx){return null;}','legacy_entry_main_missing'],
  ['function main(ctx){return null;} function main(ctx){return null;}','legacy_entry_main_ambiguous'],
  ['const main=42;','entry_main_noncallable'],
  ['async function main(ctx){return null;}','entry_main_async'],
  ['function* main(ctx){yield 1;}','entry_main_generator'],
 ]) {const result=inspect(text);assert.equal(result.passed,false,text);assert(codes(result).includes(code),JSON.stringify(result));assert.equal(result.staticAnalysisComplete,false);assert(result.diagnostics.every(d=>d.file==='nested/work.js'&&d.line>=1&&d.column>=1));}
});
test('非常规签名、别名、条件声明与顶层返回明确不完整',()=>{
 for(const text of ['function main(){return null;}','function main(...args){return null;}',
  'function main({input}){return input;}','function run(ctx){return ctx.input;}const main=run;',
  'if(true){function main(ctx){return ctx.input;}}',
  'function main(ctx){return ctx.input;}if(flag)return null;']) {
  const result=inspect(text);assert.equal(result.passed,true,JSON.stringify(result));assert.equal(result.staticAnalysisComplete,false);assert(result.diagnostics.some(d=>d.severity==='warning'));
 }
});
test('main重绑定及解构写入不报告完整，局部同名参数不误判',()=>{
 for(const text of ['function main(ctx){return ctx.input;}main=other;',
  'function main(ctx){return ctx.input;}function change(){main=other;}',
  'function main(ctx){return ctx.input;}({main}=other);',
  'function main(ctx){return ctx.input;}[main]=other;',
  'function main(ctx){return ctx.input;}function change(){for(main of values){}}',
  'function main(ctx){return ctx.input;}function change(){for(main in values){}}',
  'function main(ctx){return ctx.input;}function change(){main++;}']) {
  const result=inspect(text);assert.equal(result.staticAnalysisComplete,false);assert(codes(result).includes('legacy_entry_main_reassigned'),JSON.stringify(result));
 }
 const shadow=inspect('function helper(main){main=1;return main;}function main(ctx){return ctx.input;}');
 assert.equal(shadow.staticAnalysisComplete,true,JSON.stringify(shadow));
});
test('eval、Function、require及全局/constructor别名保留依赖不确定性，绝不执行',()=>{
 globalThis.__legacyAuditExecuted=false;
 for(const expression of ["eval('globalThis.__legacyAuditExecuted=true')",
  "new Function('return 1')()",'require(ctx.input.module)',
  "globalThis['eval']('x')",'globalThis[ctx.input.name](1)',
  "(()=>{}).constructor('return 1')()",'ctx.input.tools[ctx.input.name]()']){
  const result=inspect('function main(ctx){return '+expression+';}');
  assert.equal(result.staticAnalysisComplete,false,expression);assert.equal(result.dependencyAnalysisComplete,false,JSON.stringify(result));
 }
 const init=inspect('globalThis.__legacyAuditExecuted=true;throw Error("MUST NOT RUN");function main(ctx){return ctx.input;}');
 assert.equal(init.staticAnalysisComplete,false);assert(codes(init).includes('legacy_entry_initialization_unresolved'));
  const standaloneGraph=graph('function main(ctx){return require(ctx.input.module);}');
  assert.equal(standaloneGraph.staticGraphComplete,false);assert(codes(standaloneGraph).includes('legacy_entry_dynamic_code'));
  const alias=inspect('function main(ctx){const make=Reflect.get(()=>null,"constructor");return make("return 1")();}');
  assert.equal(alias.staticAnalysisComplete,false);assert(codes(alias).includes('legacy_entry_dynamic_call_target'));
 assert.equal(globalThis.__legacyAuditExecuted,false);delete globalThis.__legacyAuditExecuted;
});
test('静态模块语法、import.meta和同步await明确拒绝，不跟踪不存在的模块',()=>{
 for(const text of ['import x from "./missing.js";function main(ctx){return x;}',
  'export function main(ctx){return null;}', 'function main(ctx){return import.meta.url;}',
  'function main(ctx){return await other();}', 'await other();function main(ctx){return null;}']) {
  const entry=inspect(text);assert.equal(entry.passed,false,text);assert.equal(entry.staticAnalysisComplete,false);
 }
 const result=graph('import x from "./missing.js";function main(ctx){return x;}');
 assert.equal(result.passed,false);assert(codes(result).includes('legacy_entry_module_syntax'));assert.deepEqual(result.edges,[]);assert.equal(result.filesRead,false);
});
test('动态导入只标记未覆盖的边，不读取或假装加载成功',()=>{
 for(const call of ['import("./later.js")','import(ctx.input.module)']) {
  const entry=inspect('function main(ctx){'+call+';return null;}');
  assert.equal(entry.staticAnalysisComplete,false);assert.equal(entry.dependencyAnalysisComplete,false);assert(codes(entry).includes('legacy_dynamic_import_unresolved'));
  const result=graph('function main(ctx){'+call+';return null;}');
  assert.equal(result.passed,true);assert.equal(result.staticGraphComplete,false);assert(codes(result).includes('legacy_dynamic_import_unresolved'));
  assert.equal(result.filesRead,false);assert.equal(result.runtimeExecuted,false);
 }
 assert.equal(graph('function main(ctx){import("./later.js");return null;}').edges[0].kind,'dynamic_literal');
});
test('明确Promise结果不能假冒同步入口，未返回的async辅助函数不一概拒绝',()=>{
 for(const text of ['function main(ctx){return Promise.resolve(null);}',
  'function main(ctx){return new Promise(()=>{});}',
  'function main(ctx){const value=Promise.resolve(null);return value;}',
  'async function helper(){return null;}function main(ctx){return helper();}']) {
  const result=inspect(text);assert.equal(result.passed,false);assert(codes(result).includes('entry_main_async_result'),JSON.stringify(result));
 }
 const helper=inspect('async function helper(){await other();}function main(ctx){return ctx.input;}');
 assert.equal(helper.staticAnalysisComplete,true,JSON.stringify(helper));
});
test('注释、字符串及JSDoc类型引用不是运行时模块依赖，TS语法和预算仍拒绝',()=>{
 const text='/** @param {import("./types.js").Ctx} ctx */\nfunction main(ctx){const value="eval(1);import(1)";return {value};}';
 const valid=inspect(text);assert.equal(valid.staticAnalysisComplete,true,JSON.stringify(valid));assert.deepEqual(graph(text).edges,[]);
  const data=inspect('const value={eval:1,Function:"data"};function main(ctx){return value;}');
  assert.equal(data.staticAnalysisComplete,true,JSON.stringify(data));
  const shadow=inspect('function eval(value){return value;}function main(ctx){return eval(ctx.input);}');
  assert.equal(shadow.staticAnalysisComplete,true,JSON.stringify(shadow));
 for(const source of ['function main(ctx:any){return null;}','interface X{}function main(ctx){return null;}'])assert.equal(inspect(source).passed,false);
 assert.equal(inspect('x'.repeat(128*1024+1)).diagnostics[0].code,'entry_snapshot_invalid');
});
