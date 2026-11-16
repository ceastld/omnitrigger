import assert from 'node:assert/strict';
import {mkdirSync, mkdtempSync, readFileSync, writeFileSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import ts from 'typescript';

// 只取实际检查函数；不执行检查器的应用 CLI 或任何作者入口。
const source = ts.createSourceFile('check.mjs', readFileSync(new URL('./check.mjs', import.meta.url), 'utf8'),
  ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
const declaration = source.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'diagnostics');
assert(declaration);
const diagnostics = Function('assert', 'join', 'ts', `return (${declaration.getText(source)});`)(assert, join, ts);
const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const parent = join(repo, '.tmp/compiler-config-location');
mkdirSync(parent, {recursive:true});
const root = mkdtempSync(join(parent, 'case-'));

function project(name, types, withNode) {
  const directory = join(root, name);
  mkdirSync(directory);
  writeFileSync(join(directory, 'jsconfig.json'), JSON.stringify({compilerOptions:{allowJs:true, checkJs:true,
    strict:true, noEmit:true, target:'ES2022', module:'ESNext', moduleResolution:'Bundler', lib:['ES2022'], types}, files:['main.mjs']}));
  writeFileSync(join(directory, 'main.mjs'), "export const bytes = Buffer.byteLength('synthetic', 'utf8');\n");
  if (withNode) {
    const library = join(directory, 'node_modules/@types/node');
    mkdirSync(library, {recursive:true});
    writeFileSync(join(library, 'index.d.ts'), 'declare const Buffer: {byteLength(value:string, encoding:string):number};\n');
  }
  return directory;
}

test('通用检查器从作者配置位置找到工程自己的显式Node类型', () => {
  assert.equal(diagnostics(project('node', ['node'], true)).length, 0);
});

test('相邻轻量工程不继承Node全局，缺少声明的工程保留类型缺失错误', () => {
  diagnostics(project('another-node', ['node'], true));
  assert(diagnostics(project('script', [], false)).some(item => [2304, 2591].includes(item.code)));
  assert(diagnostics(project('missing-node', ['node'], false)).some(item => item.code === 2688));
});
