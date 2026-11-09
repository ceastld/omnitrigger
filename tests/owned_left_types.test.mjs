// TypeScript严格noEmit，虚拟源码；不创建工程、不导入/执行作者脚本或宿主。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import {dirname,resolve,sep} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
const require=createRequire(new URL('../tools/script-sdk-check/package.json',import.meta.url));
const ts=require('typescript');const tsLib=dirname(require.resolve('typescript'));
const out=dirname(fileURLToPath(import.meta.url));const virtual=resolve(out,'.virtual-types');
const files={
  'api.d.ts':readFileSync(new URL('../crates/omnitrigger-actions/runtime/api.d.ts',import.meta.url),'utf8'),
  'script-types.d.ts':readFileSync(new URL('../crates/omnitrigger-actions/runtime/script-types.d.ts',import.meta.url),'utf8'),
  'clipboard-history-api.d.ts':readFileSync(new URL('../crates/omnitrigger-actions/runtime/clipboard-history-api.d.ts',import.meta.url),'utf8'),
};

function check(text,source=files){
  const cases=typeof text==='string'?{'case.ts':text}:text;
  const contents=new Map(Object.entries({...source,...cases}).map(([name,value])=>[resolve(virtual,name),value]));
  const options={target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,moduleResolution:ts.ModuleResolutionKind.Bundler,
    strict:true,exactOptionalPropertyTypes:true,noUncheckedIndexedAccess:true,noEmit:true,skipLibCheck:false,types:[],
    lib:['lib.es2022.d.ts','lib.dom.d.ts']};
  const host=ts.createCompilerHost(options);const originalRead=host.readFile,originalExists=host.fileExists;
  const library=file=>resolve(file).startsWith(tsLib+sep);
  host.readFile=file=>contents.get(resolve(file))??(library(file)?originalRead(file):undefined);
  host.fileExists=file=>contents.has(resolve(file))||(library(file)&&originalExists(file));
  host.directoryExists=path=>resolve(path)===virtual||resolve(path)===tsLib||resolve(path).startsWith(tsLib+sep);
  host.getDirectories=()=>[];host.getCurrentDirectory=()=>virtual;host.realpath=file=>file;
  host.getSourceFile=(file,language)=>{const value=host.readFile(file);return value===undefined?undefined:ts.createSourceFile(file,value,language,true);};
  const program=ts.createProgram([...Object.keys(cases).map(name=>resolve(virtual,name)),resolve(virtual,'script-types.d.ts')],options,host);
  return ts.getPreEmitDiagnostics(program);
}
const prefix=`import type {ActionContext,BrightnessStepResult,BrightnessStepMember,BrightnessStepWmi} from './api';\ndeclare const ctx:ActionContext;\ndeclare const sync:ScriptContext;\n`;
const results=[];
test('owned-left Node与Script签名、未知效果及亮度字段并存',()=>{
  const positive=[`const down:Promise<{submitted:true;effectState:'unknown'}>=ctx.desktop.pressLeftButton(); const up:Promise<{ownedPressEnded:true;effectState:'unknown'}>=ctx.desktop.releaseLeftButton(); const m:import('./api').DesktopFault['method']='mouse.left_down';`,`const down:{submitted:true;effectState:'unknown'}=sync.desktop.pressLeftButton(); const up:{ownedPressEnded:true;effectState:'unknown'}=sync.desktop.releaseLeftButton(); declare const e:import('./api').DesktopFault; const receipt:BrightnessStepResult|undefined=e.brightnessReceipt;`];
  for(const value of positive)assert.deepEqual(check(prefix+value).map(d=>[d.code,ts.flattenDiagnosticMessageText(d.messageText,' ')]),[]);
});
test('owned-left拒绝owner/target/多参及错误异步或method形状',()=>{
  const negative={node_owner:`ctx.desktop.pressLeftButton({owner:1});`,script_target:`sync.desktop.releaseLeftButton({target:1});`,node_explicit_undefined:`ctx.desktop.pressLeftButton(undefined);`,script_explicit_undefined:`sync.desktop.releaseLeftButton(undefined);`,node_is_not_sync:`const r:{submitted:true;effectState:'unknown'}=ctx.desktop.pressLeftButton();`,script_is_not_promise:`const r:Promise<{ownedPressEnded:true;effectState:'unknown'}>=sync.desktop.releaseLeftButton();`,method_not_open_string:`const m:import('./api').DesktopFault['method']='mouse.middle_down';`};
  const diagnostics=check(Object.fromEntries(Object.entries(negative).map(([name,value])=>[name+'.ts',prefix+value])));
  for(const name of Object.keys(negative))assert(diagnostics.some(d=>d.file&&resolve(d.file.fileName)===resolve(virtual,name+'.ts')),name);
});
