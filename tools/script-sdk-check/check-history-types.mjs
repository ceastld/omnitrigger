// 只做真实 TypeScript noEmit 检查，不启动应用、宿主或读取任何历史正文。
import assert from 'node:assert/strict';
import {mkdirSync, mkdtempSync, writeFileSync} from 'node:fs';
import {dirname, join, relative, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import ts from 'typescript';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const normalized = path => path.replaceAll('\\', '/');
const allowedCodes = new Set([2322, 2339, 2345, 2353, 2554, 2739, 2740, 2741]);

function diagnostics(file) {
  const options = {noEmit:true, strict:true, skipLibCheck:false, target:ts.ScriptTarget.ES2022,
    module:ts.ModuleKind.NodeNext, moduleResolution:ts.ModuleResolutionKind.NodeNext,
    lib:['lib.es2022.d.ts','lib.dom.d.ts'], types:[]};
  const program = ts.createProgram([file], options);
  return ts.getPreEmitDiagnostics(program);
}
function report(items, root) {
  return ts.formatDiagnosticsWithColorAndContext(items, {getCanonicalFileName:path=>path,
    getCurrentDirectory:()=>root, getNewLine:()=> '\n'});
}
function importPath(directory, path) {
  const name = normalized(relative(directory, path)).replace(/\.d\.ts$/, '');
  return name.startsWith('.') ? name : './' + name;
}

export function checkClipboardHistoryTypes({
  syncDeclarationPath = join(repo, 'crates/omnitrigger-actions/runtime/script-api.d.ts'),
  asyncDeclarationPath = join(repo, 'crates/omnitrigger-actions/runtime/api.d.ts'),
  temporaryRoot,
} = {}) {
  const parent = temporaryRoot ?? join(repo, '.tmp');
  mkdirSync(parent, {recursive:true});
  const root = mkdtempSync(join(parent, 'history-types-'));
  const results = [];
  for (const asynchronous of [false, true]) {
    const mode = asynchronous ? 'node' : 'script';
    const folder = join(root, mode);
    mkdirSync(folder);
    const header = asynchronous
      ? `import type {ActionContext} from ${JSON.stringify(importPath(folder, asyncDeclarationPath))};\n`
        + `import type {ClipboardHistoryList,ClipboardHistoryText,ClipboardHistoryAppend,ClipboardHistoryRemove,ClipboardHistoryApi,AsyncClipboardHistoryApi} from ${JSON.stringify(importPath(folder,join(repo,'crates/omnitrigger-actions/runtime/clipboard-history-api.d.ts')))};\n`
        + 'declare const ctx: ActionContext;\n'
      : `/// <reference path=${JSON.stringify(normalized(relative(folder, syncDeclarationPath)))} />\n`;
    const awaitCall = expression => asynchronous ? `await ${expression}` : expression;
    const prefix = asynchronous ? '' : 'Script';
    const positive = `${header}${asynchronous ? 'async ' : ''}function verifyHistory(ctx: ${asynchronous ? 'ActionContext' : 'ScriptContext'}) {\n`
      + `const listing: ${prefix}ClipboardHistoryList = ${awaitCall('ctx.clipboard.history.list()')};\n`
      + `const limited: ${prefix}ClipboardHistoryList = ${awaitCall('ctx.clipboard.history.list({limit:200})')};\n`
      + `const item: ${prefix}ClipboardHistoryText = ${awaitCall("ctx.clipboard.history.get({id:'18446744073709551615'})")};\n`
      + `const saved: ${prefix}ClipboardHistoryAppend = ${awaitCall("ctx.clipboard.history.append({text:'  synthetic 😀  '})")};\n`
      + `const empty: ${prefix}ClipboardHistoryAppend = ${awaitCall("ctx.clipboard.history.append({text:''})")};\n`
      + `const removed: ${prefix}ClipboardHistoryRemove = ${awaitCall("ctx.clipboard.history.remove({id:'9007199254740993'})")};\n`
      + 'const revision: string = listing.revision; const id: string = saved.id; const stamp: string = item.createdAt;\n'
      + 'const units: number = listing.items[0].utf16Units; const preview: string = listing.items[0].preview;\n'
      + 'const body: string = item.text; const inserted: boolean = empty.inserted; const didRemove: boolean = removed.removed;\n'
      + (asynchronous ? 'const pending: Promise<ClipboardHistoryList> = ctx.clipboard.history.list({limit:1});\nconst api: AsyncClipboardHistoryApi = ctx.clipboard.history;\n'
        : 'const api: ScriptClipboardHistoryApi = ctx.clipboard.history;\n')
      // 标准同步入口能直接返回真实JSON回执；这里只检查类型，不执行历史操作。
      + (asynchronous ? '' : 'const savedMain: ScriptMain = value => value.clipboard.history.append({text:"synthetic"});\n'
        + 'const removedMain: ScriptMain = value => value.clipboard.history.remove({id:"1"});\n'
        + 'const listedMain: ScriptMain = value => value.clipboard.history.list({limit:1});\n'
        + 'const readMain: ScriptMain = value => value.clipboard.history.get({id:"1"});\n'
        + 'function itemAsJson(value:ScriptClipboardHistoryItem):ScriptJsonValue{return value;}\n')
      + 'return {revision,id,stamp,units,preview,inserted,didRemove,count:limited.items.length};\n}\n';
    const positivePath = join(folder, 'positive.ts');
    writeFileSync(positivePath, positive);
    const positiveDiagnostics = diagnostics(positivePath);
    assert.equal(positiveDiagnostics.length, 0, report(positiveDiagnostics, root));

    const cases = {
      ...(asynchronous ? {} : {
        jsonPromise:'const main: ScriptMain = () => Promise.resolve(true);',
        jsonFunction:'const main: ScriptMain = () => ({handler:() => true});',
      }),
      listLimitType:'ctx.clipboard.history.list({limit:"100"});',
      listExtra:'ctx.clipboard.history.list({page:1});',
      listArity:'ctx.clipboard.history.list({}, {});',
      getIdNumber:'ctx.clipboard.history.get({id:9007199254740993});',
      getMissingId:'ctx.clipboard.history.get({});',
      getExtra:'ctx.clipboard.history.get({id:"1",path:"synthetic"});',
      getArity:'ctx.clipboard.history.get();',
      appendTextType:'ctx.clipboard.history.append({text:123});',
      appendMissing:'ctx.clipboard.history.append({});',
      appendExtra:'ctx.clipboard.history.append({text:"",capture:true});',
      appendArity:'ctx.clipboard.history.append({text:""}, {});',
      removeIdNumber:'ctx.clipboard.history.remove({id:1});',
      removeMissing:'ctx.clipboard.history.remove();',
      removeExtra:'ctx.clipboard.history.remove({id:"1",all:true});',
      missingMethod:'ctx.clipboard.history.clear();',
      listHasNoBody:`const text = (${awaitCall('ctx.clipboard.history.list()')}).items[0].text;`,
      revisionIsString:`const revision: number = (${awaitCall('ctx.clipboard.history.list()')}).revision;`,
      getBodyIsString:`const body: number = (${awaitCall('ctx.clipboard.history.get({id:"1"})')}).text;`,
      appendIdIsString:`const id: number = (${awaitCall('ctx.clipboard.history.append({text:""})')}).id;`,
      appendHasNoBody:`const body = (${awaitCall('ctx.clipboard.history.append({text:""})')}).text;`,
      insertedIsBoolean:`const inserted: number = (${awaitCall('ctx.clipboard.history.append({text:""})')}).inserted;`,
      removedIsBoolean:`const removed: string = (${awaitCall('ctx.clipboard.history.remove({id:"1"})')}).removed;`,
      resultTiming: asynchronous
        ? 'const immediate: ClipboardHistoryList = ctx.clipboard.history.list();'
        : 'const pending: Promise<ScriptClipboardHistoryList> = ctx.clipboard.history.list();',
      resultAccess: asynchronous ? 'ctx.clipboard.history.get({id:"1"}).text;' : 'ctx.clipboard.history.get({id:"1"}).then(() => true);',
    };
    let negative = `${header}${asynchronous ? 'async ' : ''}function rejectHistory(ctx: ${asynchronous ? 'ActionContext' : 'ScriptContext'}) {\n`;
    const expectedLines = new Map();
    for (const [name, statement] of Object.entries(cases)) {
      negative += `// ${name}\n`;
      expectedLines.set(negative.split('\n').length - 1, name);
      negative += `{ ${statement} }\n`;
    }
    negative += '}\n';
    const negativePath = join(folder, 'negative.ts');
    writeFileSync(negativePath, negative);
    const negativeDiagnostics = diagnostics(negativePath);
    const rejected = new Set();
    for (const item of negativeDiagnostics) {
      assert(item.file && resolve(item.file.fileName) === negativePath && allowedCodes.has(item.code), report([item],root));
      const line = item.file.getLineAndCharacterOfPosition(item.start).line;
      assert(expectedLines.has(line), report([item],root));
      rejected.add(expectedLines.get(line));
    }
    assert.equal(rejected.size, Object.keys(cases).length, `${mode}: 未被真实编译器拒绝的用例 ${Object.keys(cases).filter(name=>!rejected.has(name)).join(', ')}`);
    results.push({mode, positive:true, rejectedCases:[...rejected], diagnostics:negativeDiagnostics.length});
  }
  const result = {passed:true, compilerVersion:ts.version, root, modes:results, applicationStarted:false,
    runtimeStarted:false, clipboardAccessed:false};
  writeFileSync(join(root, 'result.json'), JSON.stringify(result,null,2));
  return result;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log(JSON.stringify(checkClipboardHistoryTypes()));
}
