import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const ts=createRequire(path.join(root,'tools/script-sdk-check/check.mjs'))('typescript');
const runtime=path.join(root,'crates/omnitrigger-actions/runtime');
const history=path.join(runtime,'clipboard-history-api.d.ts'),formal=path.join(root,'crates/omnitrigger-actions/runtime/clipboard-history-api.d.ts');
const dir=fs.mkdtempSync(path.join(root,'.tmp/win32-edit-types-'));
for(const kind of ['node','sync']){
 const types=path.join(runtime,kind==='node'?'api.d.ts':'script-types.d.ts');
 const call="ctx.desktop.win32EditReplaceSelection('x',{adapter:'native-edit'})";
 const value=kind==='node'?`(await ${call})`:call;
 const cases=[
  ['valid',`const options={adapter:'native-edit'} as const;const result:${kind==='node'?"Promise<{messageProcessed:true;effectState:'unknown'}>":"{messageProcessed:true;effectState:'unknown'}"}=ctx.desktop.win32EditReplaceSelection('A\\r\\n😀',options);`,null],
  ['valid_empty',"ctx.desktop.win32EditReplaceSelection('',{adapter:'winforms-edit'});",null],
  ['missing_options',"ctx.desktop.win32EditReplaceSelection('x');",2554],
  ['bad_adapter',"ctx.desktop.win32EditReplaceSelection('x',{adapter:'auto'});",2322],
  ['missing_adapter',"ctx.desktop.win32EditReplaceSelection('x',{});",2345],
  ['bad_text',"ctx.desktop.win32EditReplaceSelection(42,{adapter:'native-edit'});",2345],
  ['extra_hwnd',"ctx.desktop.win32EditReplaceSelection('x',{adapter:'native-edit',hwnd:10});",2353],
  ['extra_argument',"ctx.desktop.win32EditReplaceSelection('x',{adapter:'native-edit'},{});",2554],
  ['no_text_receipt',`${value}.text;`,2339],
  ['no_identity_receipt',`${value}.hwnd;`,2339],
  ['no_confirmed_effect',`const state:'not_started'=${value}.effectState;`,2322],
  ['sync_async',kind==='node'?`${call}.messageProcessed;`:`${call}.then(()=>{});`,2339],
 ];
 for(const [name,body,code] of cases) await test(`${kind}: ${name}`, () => {
  const file=path.join(dir,`${kind}-${name}.ts`);const header=kind==='node'?`import type {ActionContext} from ${JSON.stringify(types.replaceAll('\\','/'))};\n`:'';
  fs.writeFileSync(file,`${header}async function verify(ctx:${kind==='node'?'ActionContext':'ScriptContext'}){${body}}\n`);
  const options={strict:true,noEmit:true,types:[],target:ts.ScriptTarget.ES2022,lib:['lib.es2022.d.ts'],skipLibCheck:false};const host=ts.createCompilerHost(options),read=host.readFile.bind(host),exists=host.fileExists.bind(host);
  host.readFile=file=>path.resolve(file)===history?read(formal):read(file);host.fileExists=file=>path.resolve(file)===history||exists(file);
  const all=ts.getPreEmitDiagnostics(ts.createProgram([types,file],options,host));const local=all.filter(d=>d.file&&path.resolve(d.file.fileName)===file),other=all.filter(d=>!d.file||path.resolve(d.file.fileName)!==file);
  assert.equal(other.length,0,ts.formatDiagnostics(other,{getCanonicalFileName:x=>x,getCurrentDirectory:()=>root,getNewLine:()=> '\n'}));
  if(code===null)assert.equal(local.length,0,local.map(d=>d.code).join(','));else assert(local.some(d=>d.code===code),`${kind}/${name}: ${local.map(d=>d.code)}`);
 });
}
