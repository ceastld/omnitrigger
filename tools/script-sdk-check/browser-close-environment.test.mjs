import assert from 'node:assert/strict';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import ts from 'typescript';

const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const project=resolve(repo,'examples/actions/browser-close'),configFile=resolve(project,'jsconfig.json');
const config=ts.readConfigFile(configFile,ts.sys.readFile);assert.equal(config.error,undefined);
const parsed=ts.parseJsonConfigFileContent(config.config,ts.sys,project,undefined,configFile);
function diagnostics(probe){
  const file=resolve(project,'__environment_probe__.ts'),host=ts.createCompilerHost(parsed.options),original=host.getSourceFile;
  if(probe!==undefined)host.getSourceFile=(name,version,...args)=>resolve(name)===file?ts.createSourceFile(file,probe,version,true):original.call(host,name,version,...args);
  const program=ts.createProgram([...parsed.fileNames,...(probe===undefined?[]:[file])],parsed.options,host);
  return ts.getPreEmitDiagnostics(program).filter(x=>x.category===ts.DiagnosticCategory.Error).map(x=>({code:x.code,file:x.file?.fileName,line:x.file&&x.start!==undefined?x.file.getLineAndCharacterOfPosition(x.start).line+1:null,message:ts.flattenDiagnosticMessageText(x.messageText,' ')}));
}
test('已有两个浏览器关闭入口与共享算法在ES2022宿主SDK环境严格通过',()=>{
  assert.deepEqual(parsed.options.lib,['lib.es2022.d.ts']);assert.deepEqual(parsed.options.types,[]);
  assert.equal(parsed.options.strict,true);assert.equal(parsed.options.noEmit,true);assert.equal(parsed.options.skipLibCheck,false);
  assert.deepEqual(diagnostics(),[]);
});
test('虚拟作者源拒绝DOM和Promise/同步误用，不执行入口',()=>{
  const errors=diagnostics(`import type {ActionContext} from './sdk/api';
declare const node:ActionContext;
window.document.title;
document.title;
const pending=node.browser.profiles();
pending.length;
declare const script:ScriptContext;
script.browser.profiles().then;
`);
  assert.equal(errors.length,4,JSON.stringify(errors));
  assert(errors.every(x=>x.file?.endsWith('__environment_probe__.ts')));
  assert(errors.some(x=>x.line===3&&x.code===2304));
  assert(errors.some(x=>x.line===4&&[2304,2584].includes(x.code)));
  assert(errors.some(x=>x.line===6&&x.code===2339));
  assert(errors.some(x=>x.line===8&&x.code===2339));
});
