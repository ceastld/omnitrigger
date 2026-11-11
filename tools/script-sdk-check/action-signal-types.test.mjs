import assert from 'node:assert/strict';
import test from 'node:test';
import {createRequire} from 'node:module';
import {mkdtempSync, mkdirSync, writeFileSync, readFileSync} from 'node:fs';
import {dirname, join, relative, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import ts from 'typescript';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const parent = join(repo, '.tmp', 'signal-types');
mkdirSync(parent, {recursive: true});

function check(body, nativeEnvironment = false) {
  const root = mkdtempSync(join(parent, 'case-'));
  const entry = join(root, 'probe.ts');
  const api = relative(root, join(repo, 'crates/omnitrigger-actions/runtime/api.d.ts')).replaceAll('\\', '/');
  writeFileSync(entry, `import type {ActionContext, ActionCancellationCallback} from '${api}';\ndeclare const ctx: ActionContext;\n${body}\n`);
  const program = ts.createProgram([entry], {
    strict: true, noEmit: true, skipLibCheck: false, types: [],
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    lib: ['lib.es2022.d.ts', ...(nativeEnvironment ? ['lib.dom.d.ts'] : [])],
  });
  const diagnostics = ts.getPreEmitDiagnostics(program).filter(d => d.category === ts.DiagnosticCategory.Error);
  return {
    diagnostics,
    message: ts.formatDiagnosticsWithColorAndContext(diagnostics, {
      getCurrentDirectory: () => root, getCanonicalFileName: file => file, getNewLine: () => '\n',
    }),
    domLoaded: program.getSourceFiles().some(file => /[/\\]lib\.dom\.d\.ts$/i.test(file.fileName)),
  };
}

test('独立SDK的取消观察类型不要求环境全局', () => {
  const result = check(`
ctx.signal.throwIfAborted();
const callback: ActionCancellationCallback = function(event) {
  const type: 'abort' = event.type;
  void type; void this.aborted; void event.target?.reason;
};
ctx.signal.addEventListener('abort', callback, {once: true});
ctx.signal.removeEventListener('abort', callback);
ctx.signal.onabort = callback;
`);
  assert.equal(result.diagnostics.length, 0, result.message);
  assert.equal(result.domLoaded, false);
});

test('导入SDK不会给无环境工程添加window和document', () => {
  const result = check('window.location.href;\ndocument.title;');
  assert.equal(result.diagnostics.length, 2, result.message);
  assert(result.diagnostics.every(d => [2304, 2584].includes(d.code)), result.message);
  assert.equal(result.domLoaded, false);
});

test('作者提供原生环境类型后信号保留原生赋值与函数参数兼容', () => {
  const result = check(`
const nativeSignal: AbortSignal = ctx.signal;
const fromNative: ActionContext['signal'] = new AbortController().signal;
const callback: ActionCancellationCallback = function(event) { void this.aborted; void event.type; };
ctx.signal.addEventListener('abort', callback);
const response: Promise<Response> = fetch('https://example.invalid', {signal: ctx.signal});
void nativeSignal; void fromNative; void response;
`, true);
  assert.equal(result.diagnostics.length, 0, result.message);
  assert.equal(result.domLoaded, true);
});


test('历史SDK仅分发类型，不导出没有运行时模块的内部工厂', () => {
  const root = mkdtempSync(join(parent, 'history-type-only-'));
  mkdirSync(join(root, 'sdk'));
  writeFileSync(join(root, 'sdk/clipboard-history-api.d.ts'),
    readFileSync(join(repo, 'crates/omnitrigger-actions/runtime/clipboard-history-api.d.ts')));
  const entry = join(root, 'probe.ts');
  writeFileSync(entry, "import {createClipboardHistoryApi} from './sdk/clipboard-history-api.js';\ncreateClipboardHistoryApi();\n");
  const program = ts.createProgram([entry], {strict:true, noEmit:true, skipLibCheck:false, types:[],
    target:ts.ScriptTarget.ES2022, module:ts.ModuleKind.ESNext,
    moduleResolution:ts.ModuleResolutionKind.Bundler, lib:['lib.es2022.d.ts']});
  const errors = ts.getPreEmitDiagnostics(program).filter(d=>d.category===ts.DiagnosticCategory.Error);
  assert.equal(errors.length, 1, '工厂只在宿主内部拼接，不是作者可导入的运行时模块');
  assert([2305,2724].includes(errors[0].code), '缺少导出；TypeScript可提示相似的接口类型名称');
  const declaration=program.getSourceFile(join(root, 'sdk/clipboard-history-api.d.ts'));
  const checker=program.getTypeChecker();
  assert(!checker.getExportsOfModule(checker.getSymbolAtLocation(declaration)).some(symbol=>symbol.name==='createClipboardHistoryApi'));
  assert.throws(()=>createRequire(entry).resolve('./sdk/clipboard-history-api.js'), error=>error.code==='MODULE_NOT_FOUND');
  const positive = check(`
const listed = ctx.clipboard.history.list({limit:1});
listed.then(value => { const revision:string=value.revision; void revision; });
ctx.clipboard.history.get({id:'1'}).then(value=>{const text:string=value.text;void text;});
ctx.clipboard.history.append({text:'synthetic'}).then(value=>{const inserted:boolean=value.inserted;void inserted;});
ctx.clipboard.history.remove({id:'1'}).then(value=>{const removed:boolean=value.removed;void removed;});
`);
  assert.equal(positive.diagnostics.length, 0, positive.message);
  assert.equal(positive.domLoaded, false);
});
