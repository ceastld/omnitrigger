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
  assert.equal(analyzeScriptEntry({file:'legacy.js',text:''}).diagnostics[0].code,'entry_snapshot_invalid');
  assert.equal(analyzeScriptEntry().passed,false);
  for(const invalid of [null,[],4])assert.equal(analyzeScriptEntry(invalid).passed,false);
});
