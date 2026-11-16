import assert from 'node:assert/strict';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import ts from 'typescript';
const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const project=resolve(repo,'examples/actions/desktop-line-edit'),file=resolve(project,'jsconfig.json');
const config=ts.readConfigFile(file,ts.sys.readFile);assert.equal(config.error,undefined);
const parsed=ts.parseJsonConfigFileContent(config.config,ts.sys,project,undefined,file);
function diagnostics(probe){
  const virtual=resolve(project,'__environment_probe__.ts'),host=ts.createCompilerHost(parsed.options),original=host.getSourceFile;
  if(probe!==undefined)host.getSourceFile=(name,version,...args)=>resolve(name)===virtual?ts.createSourceFile(virtual,probe,version,true):original.call(host,name,version,...args);
  const program=ts.createProgram([...parsed.fileNames,...(probe===undefined?[]:[virtual])],parsed.options,host);
  return ts.getPreEmitDiagnostics(program).filter(d=>d.category===ts.DiagnosticCategory.Error).map(d=>({code:d.code,file:d.file?.fileName,line:d.file&&d.start!==undefined?d.file.getLineAndCharacterOfPosition(d.start).line+1:null}));
}
test('已有Node入口与旧Script函数体都在显式ES2022宿主环境严格检查',()=>{
  assert.deepEqual(parsed.options.lib,['lib.es2022.d.ts']);assert.deepEqual(parsed.options.types,[]);
  assert.equal(parsed.options.strict,true);assert.equal(parsed.options.noEmit,true);assert.equal(parsed.options.skipLibCheck,false);
  assert(parsed.fileNames.some(name=>name.endsWith('/main.js')));assert(parsed.fileNames.some(name=>name.endsWith('/main.mjs')));
  assert.deepEqual(diagnostics(),[]);
});
test('虚拟作者源拒绝DOM、Node全局及目标快照的同步异步误用',()=>{
  const errors=diagnostics(`import type {ActionContext} from './sdk/api';
declare const node:ActionContext;
declare const script:ScriptContext;
window.document.title;
document.title;
process.pid;
node.desktop.target().processName;
script.desktop.target().then;
`);
  assert.equal(errors.length,5,JSON.stringify(errors));
  assert(errors.every(d=>d.file?.endsWith('__environment_probe__.ts')));
  for(const line of [4,5,6])assert(errors.some(d=>d.line===line&&[2304,2580,2584,2591].includes(d.code)));
  for(const line of [7,8])assert(errors.some(d=>d.line===line&&d.code===2339));
});
