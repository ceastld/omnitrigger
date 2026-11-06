import assert from 'node:assert/strict';
import test from 'node:test';
import {mkdtempSync, mkdirSync, writeFileSync} from 'node:fs';
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
