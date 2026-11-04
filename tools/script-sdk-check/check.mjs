// 仅开发与 CI 使用；不进入应用或脚本宿主发行内容。
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import ts from 'typescript';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const binary = resolve(process.argv[2] ?? 'target/debug/omnitrigger.exe');
mkdirSync(join(repo, '.tmp'), {recursive:true});
const root = mkdtempSync(join(repo, '.tmp/script-sdk-check-'));
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
  const parsed = ts.parseJsonConfigFileContent(loaded.config, ts.sys, path);
  assert.deepEqual(parsed.errors, []);
  const program = ts.createProgram(parsed.fileNames, parsed.options);
  return ts.getPreEmitDiagnostics(program);
}
function report(items) {
  return ts.formatDiagnosticsWithColorAndContext(items, {getCanonicalFileName:x=>x,getCurrentDirectory:()=>root,getNewLine:()=> '\n'});
}
const created = cli(['--create-script-action', 'sdk-test']);
assert.equal(created.status, 'created');
assert.equal(created.schema_version, 1);
const project = created.project;
assert.equal(project.package_root, join(root,'actions','sdk-test'));
const checked = diagnostics(project.package_root);
assert.equal(checked.length, 0, report(checked));
assert.equal(readFileSync(project.sdk_path,'utf8'), readFileSync(join(repo,'crates/omnitrigger-actions/runtime/script-api.d.ts'),'utf8'));
const original = readFileSync(project.entry_path,'utf8');
// 编译器必须实际拒绝错误参数和未提供的环境能力，而不只是成功解析声明。
writeFileSync(project.entry_path, `${original}\nctx.desktop.sendText(123);\nctx.browser.missing();\nprocess.exit(0);\ndocument.title;\n`);
const negative = diagnostics(project.package_root);
assert(negative.some(item=>item.code === 2345), report(negative));
assert(negative.some(item=>item.code === 2339), report(negative));
const messages = negative.map(item=>ts.flattenDiagnosticMessageText(item.messageText,'\n')).join('\n');
assert(messages.includes('process') && messages.includes('document'), messages);
writeFileSync(project.entry_path, `${original}\n// 保留作者修改\n`);
const files = ['main.js','action.json','call.json','jsconfig.json','README.md','sdk/api.d.ts'];
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
const result = {passed:true,root,compilerVersion:ts.version,positiveTypeCheck:true,negativeTypeChecks:negative.length,existingFilesPreserved:true,invalidCliDidNotCreateProject:true,personalBindingsModified:false};
writeFileSync(join(root,'result.json'), JSON.stringify(result,null,2));
console.log(JSON.stringify(result));
