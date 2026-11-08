// 仅开发与 CI 使用；不进入应用或脚本宿主发行内容。
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync} from 'node:fs';
import {basename, dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const binaryArgs = process.argv.slice(2);
if (binaryArgs.length !== 1 || !binaryArgs[0].trim() || binaryArgs[0].startsWith('--')) {
  console.error('用法：node tools/script-sdk-check/check.mjs <与当前 SDK 匹配的本次构建应用路径>');
  process.exit(2);
}
const binary = resolve(binaryArgs[0]);
const {default: ts} = await import('typescript');
const {checkClipboardHistoryTypes} = await import('./check-history-types.mjs');
const {checkFileDirectoryTypes} = await import('./check-file-directory-types.mjs');
const {checkActionHint} = await import('./check-action-hint.mjs');
mkdirSync(join(repo, '.tmp'), {recursive:true});
const root = mkdtempSync(join(repo, '.tmp/script-sdk-check-'));
const actionHintChecks = await checkActionHint({typescript:ts,temporaryRoot:root});
const config = join(root, 'bindings.json');
writeFileSync(config, '[]');
function cli(args, code = 0) {
  const result = spawnSync(binary, ['--config', config, ...args], {encoding:'utf8',windowsHide:true,timeout:30000,maxBuffer:4*1024*1024});
  assert.ifError(result.error);
  assert.equal(result.status, code, result.stderr || result.stdout);
  return JSON.parse(code === 0 ? result.stdout : result.stderr);
}
function diagnostics(path) {
  const configPath = join(path, 'jsconfig.json');
  const loaded = ts.readConfigFile(configPath, ts.sys.readFile);
  assert.equal(loaded.error, undefined);
  const parsed = ts.parseJsonConfigFileContent(loaded.config, ts.sys, path, undefined, configPath);
  assert.deepEqual(parsed.errors, []);
  const program = ts.createProgram(parsed.fileNames, parsed.options);
  return ts.getPreEmitDiagnostics(program);
}
function report(items) {
  return ts.formatDiagnosticsWithColorAndContext(items, {getCanonicalFileName:x=>x,getCurrentDirectory:()=>root,getNewLine:()=> '\n'});
}
const examplesRoot = join(repo, 'examples/actions');
const checkedExamples = [];
for (const entry of readdirSync(examplesRoot, {withFileTypes:true})) {
  if (!entry.isDirectory() || entry.isSymbolicLink()) continue;
  const directory = join(examplesRoot, entry.name);
  if (!existsSync(join(directory, 'jsconfig.json'))) continue;
  const items = diagnostics(directory);
  assert.equal(items.length, 0, report(items));
  checkedExamples.push(entry.name);
}
const created = cli(['--create-script-action', 'sdk-test']);
assert.equal(created.status, 'created');
assert.equal(created.schema_version, 1);
const project = created.project;
assert.equal(project.package_root, join(root,'actions','sdk-test'));
const checked = diagnostics(project.package_root);
assert.equal(checked.length, 0, report(checked));
assert.equal(readFileSync(project.sdk_path,'utf8'), readFileSync(join(repo,'crates/omnitrigger-actions/runtime/script-types.d.ts'),'utf8'));
const historyTypeChecks = checkClipboardHistoryTypes({syncDeclarationPath:project.sdk_path, temporaryRoot:root});
const fileDirectoryTypeChecks = checkFileDirectoryTypes({typescript:ts, syncDeclarationPath:project.sdk_path, temporaryRoot:root});
const original = readFileSync(project.entry_path,'utf8');
// ESM 模块没有隐式上下文；只允许入口参数或显式传参，防止编辑器补全误导作者。
for (const source of [
  'ctx.delay(0); export function main() { return null; }',
  'export function helper() { return ctx.input.text; }',
]) {
  writeFileSync(project.entry_path, source);
  const items = diagnostics(project.package_root);
  assert(items.some(item => item.code === 2304 && item.file?.text.slice(item.start,item.start+item.length) === 'ctx'),report(items));
}
writeFileSync(project.entry_path, original);
// 使用新建工程实际附带的声明，验证入口签名，而不在测试中另抄一份类型。
const entryHeader = '/// <reference path="./sdk/api.d.ts" />\n/** @type {ScriptMain} */\n';
function checkEntry(source, header = entryHeader) {
  writeFileSync(project.entry_path, header + (project.entry_path.endsWith('.mjs') ? 'export ' : '') + source);
  return diagnostics(project.package_root);
}
for (const source of [
  'function main(ctx) { return {text: String(ctx.input.text), nested: [null, true, {count: 3}], invocation: ctx.invocation}; }',
  'function main(ctx) { if (ctx.input.fail) throw new Error("failure"); return null; }',
  'function main(ctx) { const value = ctx.clipboard.readData(); return value.kind === "files" ? {count:value.paths.length, sequence:value.sequenceNumber} : {units:value.text.length, sequence:value.sequenceNumber}; }',
  'function main(ctx) { const value = ctx.desktop.copySelectionData({timeoutMs:400,pollIntervalMs:10,settleMs:0,allowUnchanged:false}); return {kind:value.kind,changed:value.changed}; }',
  'function main(ctx) { return Promise.resolve({text:String(ctx.input.text)}); }',
]) {
  const items = checkEntry(source);
  assert.equal(items.length, 0, report(items));
}
const asyncEntryHeader = entryHeader.replace('{ScriptMain}', '{ScriptAsyncMain}');
const asyncEntryItems = checkEntry('async function main(ctx) { return {text:String(ctx.input.text)}; }', asyncEntryHeader);
assert.equal(asyncEntryItems.length, 0, report(asyncEntryItems));
const invalidEntries = {
  asyncWithoutAsyncType: 'async function main(ctx) { return {text: String(ctx.input.text)}; }',
  promiseNonJson: 'function main() { return Promise.resolve({handler:()=>true}); }',
  promiseUndefined: 'function main() { return Promise.resolve(undefined); }',
  missingReturn: 'function main(ctx) { String(ctx.input.text); }',
  partialReturn: 'function main(ctx) { if (ctx.input.text) return "text"; }',
  undefined: 'function main() { return undefined; }',
  nestedFunction: 'function main() { return {handler: () => true}; }',
  nestedPromise: 'function main() { return [Promise.resolve(1)]; }',
  platformObject: 'function main() { return new Map([["key", "value"]]); }',
};
for (const [name, source] of Object.entries(invalidEntries)) {
  const items = checkEntry(source);
  assert(items.some(item => [1065, 2322, 2355, 2366].includes(item.code)
    && item.file && resolve(item.file.fileName) === resolve(project.entry_path)), `${name}: ${report(items)}`);
}
// 输入泛型只提供作者字段检查，不改变运行时参数或最终JSON结果契约。
const typedInputHeader = '/// <reference path="./sdk/api.d.ts" />\n/** @typedef {{text:string,repeats:number,suffix?:string}} MyInput */\n/** @type {ScriptMain<MyInput>} */\n';
const typedInputPositive = 'function main(ctx) { return format(ctx); }\n/** @param {ScriptContext<MyInput>} ctx */\nfunction format(ctx) { return {text:ctx.input.text.repeat(ctx.input.repeats)+(ctx.input.suffix ?? "")}; }';
const typedInputItems = checkEntry(typedInputPositive, typedInputHeader);
assert.equal(typedInputItems.length, 0, report(typedInputItems));
for (const [source, header] of [
  ['function main(ctx) { return Promise.resolve(ctx.input.text); }', typedInputHeader],
  ['async function main(ctx) { return ctx.input.text; }', typedInputHeader.replace('{ScriptMain<MyInput>}', '{ScriptAsyncMain<MyInput>}')],
]) {
  const items = checkEntry(source, header);
  assert.equal(items.length, 0, report(items));
}
const typedInputInvalid = {
  misspelledField: 'function main(ctx) { return ctx.input.txet; }',
  fieldType: 'function main(ctx) { ctx.input.repeats = "twice"; return null; }',
  asyncWithoutAsyncType: 'async function main(ctx) { return ctx.input.text; }',
  promiseNonJson: 'function main(ctx) { return Promise.resolve({callback:()=>ctx.input.text}); }',
  nonJsonReturn: 'function main(ctx) { return {callback:()=>ctx.input.text}; }',
};
for (const [name, source] of Object.entries(typedInputInvalid)) {
  const items = checkEntry(source, typedInputHeader);
  const expected = name === 'misspelledField' ? [2339,2551] : name === 'fieldType' ? [2322] : [1065,2322,2355,2366];
  assert(items.some(item => expected.includes(item.code)
    && item.file && resolve(item.file.fileName) === resolve(project.entry_path)), `${name}: ${report(items)}`);
  if (name === 'misspelledField') assert(items.some(item => item.file
    && item.file.text.slice(item.start,item.start+item.length) === 'txet'), report(items));
}
const typedInputChecks = {positive:true, rejected:Object.keys(typedInputInvalid)};

writeFileSync(project.entry_path, `${original}\n/** @param {ScriptContext} ctx */\nfunction verifyHost(ctx) { ctx.delay(0); ctx.desktop.windowAction('minimize'); ctx.desktop.windowAction('toggle_topmost'); }\n`);
const withDelay = diagnostics(project.package_root);
assert.equal(withDelay.length, 0, report(withDelay));
// 编译器必须实际拒绝错误参数和未提供的环境能力，而不只是成功解析声明。
writeFileSync(project.entry_path, `${original}\n/** @param {ScriptContext} ctx */\nfunction rejectHost(ctx) {\nctx.desktop.sendText(123);\nctx.delay('10');\nctx.desktop.windowAction('resize');\nctx.browser.missing();\nprocess.exit(0);\ndocument.title;\n}\n`);
const negative = diagnostics(project.package_root);
assert(negative.some(item=>item.code === 2345), report(negative));
assert(negative.some(item=>item.code === 2345 && item.file?.text.slice(item.start, item.start + item.length) === "'10'"), report(negative));
assert(negative.some(item=>item.code === 2345 && item.file?.text.slice(item.start, item.start + item.length) === "'resize'"), report(negative));
assert(negative.some(item=>item.code === 2339), report(negative));
const messages = negative.map(item=>ts.flattenDiagnosticMessageText(item.messageText,'\n')).join('\n');
assert(messages.includes('process') && messages.includes('document'), messages);
for (const source of [
  'function main(ctx) { return ctx.clipboard.readData().paths.length; }',
  'function main(ctx) { return ctx.desktop.copySelectionData({timeoutMs:"400"}); }',
  'function main(ctx) { return ctx.desktop.copySelectionData({allowUnchanged:"yes"}); }',
  'function main(ctx) { return ctx.desktop.copySelectionData({unknown:true}); }',
]) {
  const items = checkEntry(source);
  assert(items.some(item => [2322,2339,2345,2353].includes(item.code)
    && item.file && resolve(item.file.fileName) === resolve(project.entry_path)), report(items));
}
writeFileSync(project.entry_path, `${original}\n// 保留作者修改\n`);
const files = [basename(project.entry_path),'action.json','call.json','jsconfig.json','README.md','sdk/api.d.ts'];
if (existsSync(join(project.package_root,'lib/text.js'))) files.push('lib/text.js');
const hashes = () => files.map(name=>createHash('sha256').update(readFileSync(join(project.package_root,name))).digest('hex'));
const before = hashes();
assert.equal(cli(['--create-script-action','sdk-test'],1).error.code, 'project_exists');
assert.deepEqual(hashes(), before);
assert.equal(cli(['--create-script-action','must-not-create','--run-action',project.call_path],1).error.code, 'invalid_cli_arguments');
assert.equal(cli(['--create-script-action','must-not-create','--check-system-controls'],1).error.code, 'invalid_cli_arguments');
assert(!existsSync(join(root,'actions','must-not-create')));
const listed = cli(['--list-package-actions']);
assert.equal(listed.actions.length, 1);
assert.equal(listed.actions[0].runtime, 'script');
assert.equal(readFileSync(config,'utf8'),'[]');
const result = {passed:true,root,compilerVersion:ts.version,positiveTypeCheck:true,checkedExamples,entryContractChecks:Object.keys(invalidEntries),typedInputChecks,negativeTypeChecks:negative.length,structuredClipboardTypeChecks:true,historyTypeChecks,fileDirectoryTypeChecks,actionHintChecks,existingFilesPreserved:true,invalidCliDidNotCreateProject:true,personalBindingsModified:false};
writeFileSync(join(root,'result.json'), JSON.stringify(result,null,2));
console.log(JSON.stringify(result));
