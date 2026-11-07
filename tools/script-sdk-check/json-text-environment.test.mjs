import assert from 'node:assert/strict';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import ts from 'typescript';
const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const project=resolve(repo,'examples/actions/json-text'),file=resolve(project,'jsconfig.json');
const config=ts.readConfigFile(file,ts.sys.readFile);assert.equal(config.error,undefined);
const parsed=ts.parseJsonConfigFileContent(config.config,ts.sys,project,undefined,file);
function diagnostics(probe){
  const virtual=resolve(project,'__environment_probe__.ts'),host=ts.createCompilerHost(parsed.options),original=host.getSourceFile;
  if(probe!==undefined)host.getSourceFile=(name,version,...args)=>resolve(name)===virtual?ts.createSourceFile(virtual,probe,version,true):original.call(host,name,version,...args);
  const program=ts.createProgram([...parsed.fileNames,...(probe===undefined?[]:[virtual])],parsed.options,host);
  return ts.getPreEmitDiagnostics(program).filter(d=>d.category===ts.DiagnosticCategory.Error).map(d=>({code:d.code,file:d.file?.fileName,line:d.file&&d.start!==undefined?d.file.getLineAndCharacterOfPosition(d.start).line+1:null}));
}
test('四个JSON入口与三个共享算法在显式ES2022宿主环境严格检查',()=>{
  assert.deepEqual(parsed.options.lib,['lib.es2022.d.ts']);assert.deepEqual(parsed.options.types,[]);
  assert.equal(parsed.options.strict,true);assert.equal(parsed.options.noEmit,true);assert.equal(parsed.options.skipLibCheck,false);
  assert.equal(parsed.fileNames.filter(name=>name.endsWith('.mjs')).length,7);
  assert.deepEqual(diagnostics(),[]);
});
test('虚拟作者源拒绝环境全局、同步异步误用和跳过JSON输入类型',()=>{
  const errors=diagnostics(`import type {NodeContext,SyncContext,SelectionInput} from './json-types';
declare const node:NodeContext;
declare const script:SyncContext;
window.document.title;
document.title;
process.pid;
node.ui.choose({items:[{id:'a',label:'A'}]}).status;
script.ui.choose({items:[{id:'a',label:'A'}]}).then;
const input:SelectionInput={mode:'inspect',text:'{}',path:'$.a',format:'compact'};
node.input.text;
`);
  assert.equal(errors.length,7,JSON.stringify(errors));
  assert(errors.every(d=>d.file?.endsWith('__environment_probe__.ts')));
  for(const line of [4,5,6])assert(errors.some(d=>d.line===line&&[2304,2580,2584,2591].includes(d.code)));
  for(const line of [7,8])assert(errors.some(d=>d.line===line&&d.code===2339));
  assert(errors.some(d=>d.line===9&&d.code===2322));
  assert(errors.some(d=>d.line===10&&d.code===18046));
});
