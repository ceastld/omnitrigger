// 既有 writeFiles / readDirectory 合同的 TypeScript noEmit 检查，不启动应用或 Native。
import assert from 'node:assert/strict';
import {mkdirSync, mkdtempSync, writeFileSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

export function checkFileDirectoryTypes({
  typescript: ts,
  syncDeclarationPath = join(repo, 'crates/omnitrigger-actions/runtime/script-types.d.ts'),
  asyncDeclarationPath = join(repo, 'crates/omnitrigger-actions/runtime/api.d.ts'),
  temporaryRoot = join(repo, '.tmp'),
  selectors = ['files','directory','logical'],
} = {}) {
  assert(ts && typeof ts.createProgram === 'function', '需要当前工具包的 TypeScript。');
  assert(Array.isArray(selectors) && selectors.length > 0 && new Set(selectors).size === selectors.length && selectors.every(name => ['files','directory','logical'].includes(name)), '未知类型检查selector');
  mkdirSync(temporaryRoot, {recursive:true});
  const root = mkdtempSync(join(temporaryRoot, 'file-directory-types-'));
  const results = [];
  for (const kind of ['node', 'script']) {
    const types = kind === 'node' ? asyncDeclarationPath : syncDeclarationPath;
    const call = "ctx.clipboard.writeFiles(['C:\\\\合成😀'])";
    const value = kind === 'node' ? `(await ${call})` : call;
    const files = [
      ['valid', `const paths=['C:\\\\a'] as const;const result:${kind === 'node' ? 'Promise<{written:true;sequenceNumber:number}>' : '{written:true;sequenceNumber:number}'}=ctx.clipboard.writeFiles(paths);`, null],
      ['missing', 'ctx.clipboard.writeFiles();', 2554],
      ['bad_array', "ctx.clipboard.writeFiles('C:\\\\a');", 2345],
      ['bad_entry', 'ctx.clipboard.writeFiles([7]);', 2322],
      ['options', "ctx.clipboard.writeFiles(['C:\\\\a'],{excludeFromHistory:true});", 2554],
      ['no_paths', `${value}.paths;`, 2339],
      ['no_paste', `${value}.pasted;`, 2339],
      ['async_sync', kind === 'node' ? `${call}.written;` : `${call}.then(()=>{});`, 2339],
    ];
    const directoryCall = options => `${kind === 'node' ? 'await ' : ''}ctx.fs.readDirectory('C:\\\\synthetic',${options})`;
    const directory = [
      ['valid-native', `const result=${directoryCall("{metadata:true,order:'native',entryType:'file',searchPattern:'*.xls',matchMode:'windows-find'}")};const ticks:string=result.entries[0].metadata.lastWriteTimeLocalTicks100ns;`, null],
      ['default', `const result=${directoryCall('{}')};const kind:string=result.entries[0].kind;`, null],
      ['default-no-metadata', `const result=${directoryCall('{}')};result.entries[0].metadata;`, 'error'],
      ['bad-mode', `${directoryCall("{searchPattern:'*',matchMode:'dotnet-managed'}")};`, 'error'],
      ['bad-order', `${directoryCall("{order:'other'}")};`, 'error'],
      ['bad-metadata', `${directoryCall("{metadata:'true'}")};`, 'error'],
      ['readonly', `const result=${directoryCall('{metadata:true}')};result.entries[0].metadata.ordinal=1;`, 'error'],
      ['precision-is-string', `const result=${directoryCall('{metadata:true}')};const ticks:number=result.entries[0].metadata.lastWriteTimeLocalTicks100ns;`, 'error'],
    ];
    const logical = [
      ['valid', "const paths:readonly string[]=['relative','','a\\0suffix'];const value:"+(kind==='node'?'Promise<readonly string[]>':'readonly string[]')+'=ctx.fs.sortLogicalPaths(paths);', null],
      ['missing', 'ctx.fs.sortLogicalPaths();', 2554],
      ['bad-array', "ctx.fs.sortLogicalPaths('x');", 2345],
      ['bad-item', 'ctx.fs.sortLogicalPaths([1]);', 2322],
      ['options', 'ctx.fs.sortLogicalPaths([],{});', 2554],
      ['readonly', 'const value=await ctx.fs.sortLogicalPaths([]);value.push("x");', 2339],
      ['timing', 'const value:'+(kind==='node'?'readonly string[]':'Promise<readonly string[]>')+'=ctx.fs.sortLogicalPaths([]);', 'error'],
    ];
    for (const [group, cases] of [['files', files], ['directory', directory], ['logical', logical]].filter(([group])=>selectors.includes(group))) {
      for (const [name, body, expected] of cases) {
        const file = join(root, `${kind}-${group}-${name}.ts`);
        const header = kind === 'node'
          ? `import type {ActionContext} from ${JSON.stringify(types.replaceAll('\\', '/'))};\n`
          : `/// <reference path=${JSON.stringify(types.replaceAll('\\', '/'))} />\n`;
        writeFileSync(file, `${header}async function verify(ctx:${kind === 'node' ? 'ActionContext' : 'ScriptContext'}){${body}}\n`);
        const options = {strict:true, noEmit:true, types:[], target:ts.ScriptTarget.ES2022,
          lib:['lib.es2022.d.ts'], skipLibCheck:false};
        const all = ts.getPreEmitDiagnostics(ts.createProgram([file], options));
        const local = all.filter(item => item.file && resolve(item.file.fileName) === file);
        const other = all.filter(item => !item.file || resolve(item.file.fileName) !== file);
        const report = items => ts.formatDiagnostics(items, {getCanonicalFileName:path=>path,
          getCurrentDirectory:()=>repo, getNewLine:()=> '\n'});
        assert.equal(other.length, 0, report(other));
        if (expected === null) assert.equal(local.length, 0, report(local));
        else if (expected === 'error') assert(local.length > 0, `${kind}/${group}/${name} 应拒绝`);
        else assert(local.some(item => item.code === expected), report(local));
        results.push({kind, group, name, expected});
      }
    }
  }
  return {passed:true, cases:results.length, noEmit:true, compilerVersion:ts.version, root};
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const {default: ts} = await import('typescript');
  console.log(JSON.stringify(checkFileDirectoryTypes({typescript:ts})));
}
