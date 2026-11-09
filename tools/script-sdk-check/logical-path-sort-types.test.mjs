// 定向类型、已发布第一方副本selectors；不运行作者、宿主或系统排序。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import ts from 'typescript';
const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
function check(runtime){
  const file=resolve(repo,'.tmp/logical-sort-'+runtime+'.ts');
  const header=runtime==='node'?"import type {ActionContext} from '../crates/omnitrigger-actions/runtime/api';declare const ctx:ActionContext;":'declare const ctx:ScriptContext;';
  const source=header+'\n'+[
    "const input:readonly string[]=['relative','','a\\0suffix','same','same'];",
    'const result:'+(runtime==='node'?'Promise<readonly string[]>':'readonly string[]')+'=ctx.fs.sortLogicalPaths(input);',
    '// @ts-expect-error 必须提供数组。','ctx.fs.sortLogicalPaths();',
    '// @ts-expect-error 不接受选项。','ctx.fs.sortLogicalPaths([],{});',
    '// @ts-expect-error 只接受字符串。','ctx.fs.sortLogicalPaths([1]);',
    '// @ts-expect-error Node/Script返回时序必须区分。','const wrong:'+(runtime==='node'?'readonly string[]':'Promise<readonly string[]>')+'=ctx.fs.sortLogicalPaths([]);',
    'async function inspect(){const value=await ctx.fs.sortLogicalPaths([]);',
    '// @ts-expect-error 回执只读。','value.push("other");',
    '// @ts-expect-error 回执只读。','value[0]="changed";}',
  ].join('\n');
  const options={strict:true,noEmit:true,skipLibCheck:false,types:[],target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,moduleResolution:ts.ModuleResolutionKind.Bundler,lib:['lib.es2022.d.ts']};
  const host=ts.createCompilerHost(options),original=host.getSourceFile.bind(host);
  host.getSourceFile=(name,...args)=>resolve(name)===file?ts.createSourceFile(name,source,options.target,true):original(name,...args);
  const roots=[file,...(runtime==='script'?[resolve(repo,'crates/omnitrigger-actions/runtime/script-types.d.ts')]:[])];
  const diagnostics=ts.getPreEmitDiagnostics(ts.createProgram(roots,options,host));
  assert.equal(diagnostics.length,0,ts.formatDiagnostics(diagnostics,{getCurrentDirectory:()=>repo,getCanonicalFileName:f=>f,getNewLine:()=> '\n'}));
}
for(const runtime of ['node','script'])test('sortLogicalPaths：'+runtime+' strict/noEmit/只读与返回时序',()=>check(runtime));
const selectors={
  'crates/omnitrigger-actions/runtime/api.d.ts':'sortLogicalPaths(paths: readonly string[]): Promise<readonly string[]>;',
  'crates/omnitrigger-actions/runtime/script-types.d.ts':'sortLogicalPaths(paths: readonly string[]): readonly string[];',
};
const copies=[
  {
    "path": "examples/actions/brightness-step/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/brightness-step/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/browser-close/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/browser-close/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/browser-workflows/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/browser-workflows/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/clipboard-history/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/clipboard-history/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/common-desktop/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/common-search/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/copy-selected-files/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/copy-selected-files/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/desktop-line-edit/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/desktop-line-edit/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/json-text/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/json-text/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/keyboard-sequence/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/keyboard-sequence/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/line-text/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/line-text/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/mixed-projects/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/mixed-projects/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/process-output/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/process-output/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/process-sequence/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/raishot/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/raishot/types/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/selection-history/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/selection-history/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/selection-line-cleanup/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/selection-line-cleanup/types/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/source-file-preview/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/volume-adjust/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/volume-adjust/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/water-reminder/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/water-reminder/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/win32-edit-line-cleanup/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  },
  {
    "path": "examples/actions/win32-edit-line-cleanup/types/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/window-adjust/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/window-size-step/sdk/api.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/api.d.ts"
  },
  {
    "path": "examples/actions/window-size-step/sdk/script-types.d.ts",
    "canonical": "crates/omnitrigger-actions/runtime/script-types.d.ts"
  }
];
test('sortLogicalPaths副本契约：41第一方声明逐字同步，并选取新方法签名',()=>{
  assert.equal(copies.length,41);
  for(const {path,canonical} of copies){
    const value=readFileSync(resolve(repo,path),'utf8').replaceAll('\r\n','\n');
    const original=readFileSync(resolve(repo,canonical),'utf8').replaceAll('\r\n','\n');
    assert.equal(value,original,path);assert.equal(value.split(selectors[canonical]).length-1,1,path);
  }
});
