import assert from 'node:assert/strict';
import test from 'node:test';
import {analyzeScriptEntry} from './analyze-script-entry.mjs';

const inspect=text=>analyzeScriptEntry({text});
test('具名函数、局部别名和箭头导出可静态找到，初始化器不执行',()=>{
  for(const text of [
    'throw Error("MUST NOT RUN");export function main(ctx){return ctx.input;}',
    'function run(){return null;}export {run as main};',
    'export const main=ctx=>ctx.input;',
    'const main=function(){return null};export {main};',
  ]) {
    const result=inspect(text);assert.equal(result.passed,true);
    assert.equal(result.mainExport,'local_callable_declaration');
    assert.equal(result.runtimeExecuted,false);assert.equal(result.runtimeVerified,false);
  }
});
test('合法JavaScript不一定有宿主需要的main具名导出',()=>{
  for(const text of ['export function other(){return null;}','function main(){return null;}',
    'export default function main(){return null;}','export default ()=>null;','// export function main(){}']) {
    const result=inspect(text);assert.equal(result.passed,false);
    assert.equal(result.mainExport,'missing_named_export');
    assert.equal(result.diagnostics[0].code,'entry_main_not_exported');
  }
});
test('明确的数字、对象或class具名导出不可直接调用',()=>{
  for(const text of ['export const main=1;','export const main={};','export class main {}',
    'export const main=false;','export * as main from "./other.js";']) {
    const result=inspect(text);assert.equal(result.passed,false);
    assert.equal(result.mainExport,'local_noncallable_declaration');
    assert.equal(result.diagnostics[0].line,1);
  }
});
test('导入别名、再导出、解构和运行时表达式保留不确定性',()=>{
  for(const text of ['import {main} from "./other.js";export {main};',
    'export {run as main} from "./other.js";','export * from "./other.js";',
    'export const {main}=getEntry();','export const main=getEntry();']) {
    const result=inspect(text);assert.equal(result.passed,true);
    assert.equal(result.mainExport,'unresolved_export');assert.equal(result.diagnostics[0].severity,'warning');
  }
});
test('主导出优先于星号，再导出未知不冒充缺失',()=>{
  const result=inspect('export * from "./other.js";export function main(){return null;}');
  assert.equal(result.mainExport,'local_callable_declaration');assert.equal(result.passed,true);
});
test('坏语法、大小和不支持的入口类型返回结构化拒绝',()=>{
  assert.equal(inspect('export function main( {').diagnostics[0].code,'entry_syntax_invalid');
  assert.equal(inspect('x'.repeat(128*1024+1)).diagnostics[0].code,'entry_snapshot_invalid');
  assert.equal(analyzeScriptEntry({file:'legacy.cjs',text:''}).diagnostics[0].code,'entry_snapshot_invalid');
  assert.equal(analyzeScriptEntry().passed,false);
  for(const invalid of [null,[],4])assert.equal(analyzeScriptEntry(invalid).passed,false);
});

test('轻量async入口即使没有ScriptMain注解也不能报告声明检查通过',()=>{
  for(const text of ['export async function main(){return null;}',
    'async function run(){return null;}export {run as main};',
    'export const main=async()=>null;',
    'const run=async function(){return null};export {run as main};']) {
    const result=inspect(text);
    assert.equal(result.passed,false);assert.equal(result.mainExport,'local_async_declaration');
    assert.equal(result.diagnostics[0].code,'entry_main_async');
    assert.equal(result.runtimeExecuted,false);assert.equal(result.filesRead,false);
  }
});
test('轻量生成器入口不能把未执行的迭代器当成JSON入口',()=>{
  for(const text of ['export function* main(){yield 1;}',
    'function* run(){yield 1;}export {run as main};',
    'export const main=function*(){return null;};',
    'export async function* main(){yield 1;}']) {
    const result=inspect(text);
    assert.equal(result.passed,false);assert.equal(result.mainExport,'local_generator_declaration');
    assert.equal(result.diagnostics[0].code,'entry_main_generator');
    assert.equal(result.diagnostics[0].severity,'error');
  }
});
test('未作为main导出的async或生成器辅助函数不被一概拒绝',()=>{
  for(const text of ['async function helper(){return null;}export function main(){return null;}',
    'function* helper(){yield 1;}export const main=()=>({ok:true});']) {
    const result=inspect(text);assert.equal(result.passed,true);
    assert.equal(result.mainExport,'local_callable_declaration');
  }
});

test('JS入口中的TS专有语法明确拒绝并定位，不自动转译',()=>{
  for(const text of [
    'export function main(ctx: any){return null;}',
    'interface Value {a: string}\nexport function main(){return null;}',
    'export function main(){return 1 as number;}',
    'import type {Value} from "./value.js"; export function main(){return null;}',
  ]) {
    const input={file:'nested/main.mjs',text:'// 中文定位\n'+text};
    const before=JSON.stringify(input),result=analyzeScriptEntry(input);
    assert.equal(result.passed,false,text);assert.equal(result.mainExport,'unresolved');
    assert(result.diagnostics.length>0,text);
    for(const item of result.diagnostics) {
      assert.equal(item.code,'entry_typescript_syntax');assert.equal(item.severity,'error');
      assert.equal(item.file,'nested/main.mjs');assert.equal(item.line,2);assert(item.column>=1);
    }
    assert.equal(result.runtimeExecuted,false);assert.equal(result.filesRead,false);assert.equal(result.filesModified,false);
    assert.equal(JSON.stringify(input),before);
  }
});
test('合法JSDoc和相对导入不影响main声明，default async辅助函数仍合法',()=>{
  const result=inspect(`import {value} from './lib/value.js';
/** @param {{input: {name: string}}} ctx */
export function main(ctx){return {name:ctx.input.name,value};}
export default async function helper(){return null;}`);
  assert.equal(result.passed,true);assert.equal(result.mainExport,'local_callable_declaration');
  assert.deepEqual(result.diagnostics,[]);
});
test('单独default async保持合法语法与既有缺少main具名导出的结论',()=>{
  const result=inspect('export default async function main(){return null;}');
  assert.equal(result.passed,false);assert.equal(result.mainExport,'missing_named_export');
  assert.deepEqual(result.diagnostics.map(item=>item.code),['entry_main_not_exported']);
});
