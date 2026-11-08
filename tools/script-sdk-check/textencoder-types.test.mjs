// 只检查宿主声明与标准库合并，不执行编码器或作者入口。
import assert from 'node:assert/strict';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import ts from 'typescript';
const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
function check(source,{script=true,dom=false,types=[],typeRoots,javascript=false}={}) {
  const file=resolve(repo,javascript?'.tmp/textencoder-contract.mjs':'.tmp/textencoder-contract.ts');
  const options={allowJs:javascript,checkJs:javascript,strict:true,exactOptionalPropertyTypes:true,noEmit:true,skipLibCheck:false,types,
    ...(typeRoots?{typeRoots}:{}),target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,
    moduleResolution:ts.ModuleResolutionKind.Bundler,lib:['lib.es2022.d.ts',...(dom?['lib.dom.d.ts']:[])]};
  const host=ts.createCompilerHost(options),original=host.getSourceFile.bind(host);
  host.getSourceFile=(name,...args)=>resolve(name)===file?ts.createSourceFile(name,source,options.target,true):original(name,...args);
  const roots=[file,...(script?[resolve(repo,'crates/omnitrigger-actions/runtime/script-types.d.ts')]:[])];
  const diagnostics=ts.getPreEmitDiagnostics(ts.createProgram(roots,options,host));
  assert.equal(diagnostics.length,0,ts.formatDiagnosticsWithColorAndContext(diagnostics,{getCurrentDirectory:()=>repo,getCanonicalFileName:f=>f,getNewLine:()=> '\n'}));
}
const encoding=[
  'const encoder:TextEncoder=new TextEncoder();',
  'const label:string=encoder.encoding;',
  'const empty:Uint8Array<ArrayBuffer>=encoder.encode();',
  'const utf8:Uint8Array<ArrayBuffer>=encoder.encode("中文😀\\ud800");',
  'const output:Uint8Array<ArrayBufferLike>=new Uint8Array(new SharedArrayBuffer(32));',
  'const receipt:TextEncoderEncodeIntoResult=encoder.encodeInto("中文",output);',
  'const read:number=receipt.read;const written:number=receipt.written;',
  '// @ts-expect-error encoding只读。',
  'encoder.encoding="other";',
  '// @ts-expect-error 输入声明为字符串。',
  'encoder.encode(123);',
  '// @ts-expect-error 目标必须是Uint8Array。',
  'encoder.encodeInto("text",new Int8Array(10));',
  'const main:ScriptMain=async ctx=>({bytes:Array.from(new TextEncoder().encode(String(ctx.input.text)))});',
  'const syncMain:ScriptMain=()=>null;',
  'const promisedMain:ScriptMain=()=>Promise.resolve({ok:true});',
  '// @ts-expect-error Promise最终结果仍必须是JSON。',
  'const invalidMain:ScriptMain=async()=>({handler:()=>true});',
].join('\n');
test('Script ES2022/types空：TextEncoder、同步和Promise入口严格通过，未扩展其它环境',()=>{
  check(encoding+'\n'+[
    '// @ts-expect-error 不注入DOM全局。','window;',
    '// @ts-expect-error 不注入DOM全局。','document;',
    '// @ts-expect-error 不注入其它编码API。','new TextDecoder();',
    '// @ts-expect-error 不注入Node全局。','process;',
    '// @ts-expect-error 不注入Node全局。','Buffer;',
    '// @ts-expect-error 不注入Node全局。','require("fs");',
  ].join('\n'));
});
test('Script TextEncoder与TypeScript DOM声明合并，无重复构造器或缓冲区类型冲突',()=>check(encoding,{dom:true}));
test('Node SDK未注入TextEncoder或其它环境全局',()=>check([
  "import type {ActionContext} from '../crates/omnitrigger-actions/runtime/api';",
  'declare const ctx:ActionContext;',
  "const pending:Promise<unknown>=ctx.browser.callAsync('tabs.get',[1]);",
  '// @ts-expect-error Node环境全局由作者显式加载@types/node。','new TextEncoder();',
  '// @ts-expect-error SDK不注入DOM全局。','window;',
  '// @ts-expect-error SDK不注入Node全局。','Buffer;',
].join('\n'),{script:false}));

test('真实JSDoc作者入口：async函数使用ScriptAsyncMain，普通函数可返回Promise',()=>{
  for(const source of [
    '/** @type {ScriptAsyncMain} */\nexport async function main(ctx){return {bytes:Array.from(new TextEncoder().encode(String(ctx.input.text)))};}',
    '/** @type {ScriptMain} */\nexport function main(ctx){return Promise.resolve({text:String(ctx.input.text)});}',
  ])check(source,{javascript:true});
});
