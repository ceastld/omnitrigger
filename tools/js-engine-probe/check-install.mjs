// 本机和 CI 的真实安装验收；只使用仓库 .tmp 下的新配置和无界面 probe，不操作输入或剪贴板。
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, unlinkSync, writeFileSync} from 'node:fs';
import {basename, dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {verifyFsStat} from '../script-sdk-check/check-fs-stat.mjs';
import {verifyFileOpen} from '../script-sdk-check/check-file-open.mjs';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const sourceApp = resolve(process.argv[2] ?? 'target/debug/omnitrigger.exe');
const packageResult = resolve(process.argv[3]);
const probeExecutable = resolve(process.argv[4] ?? join(repo,'.tmp/file-open-probe.exe'));
assert(existsSync(probeExecutable), '先编译本地 file-open-probe.rs，再向检查器传入其 EXE 路径。');
const development = JSON.parse(readFileSync(packageResult, 'utf8'));
mkdirSync(join(repo, '.tmp'), {recursive:true});
const root = mkdtempSync(join(repo, '.tmp/script-install-check-'));
// 验收期间不占用 Cargo 输出，避免 Windows 上后续构建无法替换应用文件。
const app = join(root,'application.exe');
cpSync(sourceApp,app,{errorOnExist:true});
const config = join(root, 'bindings.json');
writeFileSync(config, '[]');
const env = {...process.env, OMNITRIGGER_NODE:join(root, 'missing-node.exe')};
delete env.OMNITRIGGER_SCRIPT_HOST;
const hash = value => createHash('sha256').update(value).digest('hex');
const pointer = join(root, 'runtimes/script/current.json');
const checks = [];
function cli(args, code=0, profileArgs=['--config',config]) {
  const result = spawnSync(app, [...profileArgs,...args], {encoding:'utf8',windowsHide:true,timeout:120000,maxBuffer:1024*1024,env});
  assert.ifError(result.error);
  assert.equal(result.status, code, result.stderr || result.stdout);
  return JSON.parse(code===0 ? result.stdout : result.stderr);
}
function install(archive, code=0, digest=hash(readFileSync(archive))) {
  return cli(['--install-script-runtime',archive,'--sha256',digest],code);
}
function variant(name, change, extra='') {
  const folder = join(root,name);
  cpSync(join(dirname(packageResult),'package'),folder,{recursive:true,errorOnExist:true});
  change(folder);
  const archive = join(root,`${name}.zip`);
  // Windows PowerShell 的旧 .NET ZipFile 可能写反斜线；显式创建标准 ZIP 路径。
  const script = `$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip=[IO.Compression.ZipFile]::Open($env:TEST_ARCHIVE,[IO.Compression.ZipArchiveMode]::Create)
try {
  foreach ($file in Get-ChildItem -LiteralPath $env:TEST_SOURCE -Recurse -File) {
    $name=$file.FullName.Substring($env:TEST_SOURCE.Length+1).Replace('\\','/')
    $entry=$zip.CreateEntry($name)
    $inputStream=[IO.File]::OpenRead($file.FullName); $outputStream=$entry.Open()
    try { $inputStream.CopyTo($outputStream) } finally { $inputStream.Dispose(); $outputStream.Dispose() }
  }
  if ($env:TEST_EXTRA) { $entry=$zip.CreateEntry($env:TEST_EXTRA); $stream=$entry.Open(); $stream.WriteByte(0); $stream.Dispose() }
} finally { $zip.Dispose() }`;
  const packed = spawnSync('powershell.exe',['-NoLogo','-NoProfile','-NonInteractive','-Command',script],{encoding:'utf8',windowsHide:true,timeout:30000,env:{...env,TEST_SOURCE:folder,TEST_ARCHIVE:archive,TEST_EXTRA:extra}});
  assert.ifError(packed.error);
  assert.equal(packed.status,0,packed.stderr);
  return archive;
}
function updateManifest(folder, change) {
  const path = join(folder,'manifest.json');
  const manifest = JSON.parse(readFileSync(path,'utf8'));
  change(manifest);
  writeFileSync(path,JSON.stringify(manifest));
}

assert.equal(install(development.archive).status,'installed');
const firstPointer = readFileSync(pointer,'utf8');
const first = JSON.parse(firstPointer);
const firstHost = join(root,'runtimes/script/packages',first.archive_sha256,'host.exe');
const originalHost = readFileSync(firstHost);
checks.push('real_package_install_and_JS_probe');
const project = cli(['--create-script-action','installed-script']).project;
const outcome = cli(['--run-action',project.call_path]);
assert.equal(outcome.status,'succeeded');
assert.equal(outcome.output.text,'OmniTrigger');
checks.push('generated_project_runs_without_Node_or_manual_host');
verifyFsStat({root,entryPath:project.entry_path,call:project.call,invoke:callPath=>cli(['--run-action',callPath])});
checks.push('installed_host_fs_stat_uses_real_native_metadata_without_UI');
await verifyFileOpen({root,entryPath:project.entry_path,call:project.call,invoke:callPath=>cli(['--run-action',callPath]),probeExecutable});
checks.push('installed_host_actual_file_open_starts_owned_no_ui_probe');
assert.equal(install(development.archive).status,'installed');
assert.equal(readFileSync(pointer,'utf8'),firstPointer);
checks.push('same_package_reinstall');

for (const args of [
  ['--install-script-runtime',development.archive],
  ['--sha256',development.sha256,'--create-script-action','forbidden'],
  ['--install-script-runtime',development.archive,'--sha256',development.sha256,'--check-system-controls'],
]) assert.equal(cli(args,1).error.code,'invalid_cli_arguments');
assert(!existsSync(join(root,'actions/forbidden')));
checks.push('invalid_CLI_rejected_before_side_effects');
assert.equal(install(development.archive,1,'0'.repeat(64)).error.code,'runtime_install_failed');

const damaged = variant('damaged',folder=>writeFileSync(join(folder,'sdk/api.d.ts'),'changed'));
const wrongEngine = variant('wrong-engine',folder=>updateManifest(folder,m=>m.engine='boa'));
const wrongExecutable = variant('wrong-executable',folder=>{
  const data=Buffer.from('not executable');
  writeFileSync(join(folder,'host.exe'),data);
  updateManifest(folder,m=>Object.assign(m.files.find(f=>f.path==='host.exe'),{size:data.length,sha256:hash(data)}));
});
const traversal = variant('traversal',()=>{},'../outside.txt');
const duplicate = variant('duplicate',()=>{},'readme.TXT');
for (const [archive,reason] of [[damaged,/文件校验失败/],[wrongEngine,/自述与包清单/],[wrongExecutable,/宿主能力探测无法启动/],[traversal,/解压校验失败/],[duplicate,/解压校验失败/]]) {
  const failure=install(archive,1);
  assert.equal(failure.error.code,'runtime_install_failed');
  assert.match(failure.error.message,reason);
  assert.equal(readFileSync(pointer,'utf8'),firstPointer);
  assert.equal(hash(readFileSync(firstHost)),hash(originalHost));
}
assert(!readdirSync(join(root,'runtimes/script'),{recursive:true}).some(name=>basename(name)==='outside.txt'));
checks.push('hash_layout_duplicate_identity_and_startup_failures_preserve_active_host');

const changed = Buffer.from(originalHost);
changed[changed.length-1] ^= 1;
try {
  writeFileSync(firstHost,changed);
  const failure = cli(['--run-action',project.call_path],1);
  assert.equal(failure.error.code,'runtime_integrity');
  assert.equal(failure.error.effect_state,'not_started');
} finally { writeFileSync(firstHost,originalHost); }
checks.push('same_size_installed_image_tampering_rejected_before_execution');

const legacyHost=join(root,'runtimes/script/host.exe');
cpSync(firstHost,legacyHost,{errorOnExist:true});
try {
  writeFileSync(pointer,JSON.stringify({...first,manifest_sha256:'0'.repeat(64)}));
  const failure=cli(['--run-action',project.call_path],1);
  assert.equal(failure.error.code,'runtime_unavailable');
} finally { writeFileSync(pointer,firstPointer); unlinkSync(legacyHost); }
checks.push('invalid_managed_pointer_does_not_fall_back');

const copy = cli(['--profile-create','runtime-copy','--seed','copy:default'],0,['--data-root',root]);
const copiedArgs = ['--data-root',root,'--profile','runtime-copy'];
assert(!existsSync(join(copy.directory,'runtimes')));
assert.equal(cli(['--run-action',project.call_path],0,copiedArgs).output.text,'OmniTrigger');
const copiedPackage = join(copy.directory,'actions','installed-script');
const copiedManifest = JSON.parse(readFileSync(join(copiedPackage,'action.json'),'utf8'));
const copiedEntry = join(copiedPackage,copiedManifest.actions[0].entry);
writeFileSync(copiedEntry,'export function main() { return {text:"copied profile"}; }');
assert.equal(cli(['--run-action',project.call_path],0,copiedArgs).output.text,'copied profile');
assert.equal(cli(['--run-action',project.call_path]).output.text,'OmniTrigger');
const nested = cli(['--profile-create','nested','--seed','copy:default'],0,['--data-root',copy.directory]);
const nestedArgs = ['--data-root',copy.directory,'--profile',nested.id];
const nestedEntry = join(nested.directory,'actions','installed-script',copiedManifest.actions[0].entry);
writeFileSync(nestedEntry,'export function main() { return {text:"nested copy"}; }');
assert.equal(cli(['--run-action',project.call_path],0,nestedArgs).output.text,'nested copy');
assert(!existsSync(join(nested.directory,'runtimes')));
const copiedRuntime = join(copy.directory,'runtimes','script');
const fromFixture = cli(['--profile-create','runtime-fixture','--seed',`fixture:${root}`],0,['--data-root',root]);
const fixtureArgs = ['--data-root',root,'--profile',fromFixture.id];
assert.equal(cli(['--run-action',project.call_path],1,fixtureArgs).error.code,'runtime_unavailable');
const bareRuntime = join(fromFixture.directory,'runtimes','script');
mkdirSync(bareRuntime,{recursive:true});
cpSync(firstHost,join(bareRuntime,'host.exe'));
const bareCopy = cli(['--profile-create','runtime-bare-copy','--seed',`copy:${fromFixture.id}`],0,['--data-root',root]);
assert.equal(cli(['--run-action',project.call_path],1,['--data-root',root,'--profile',bareCopy.id]).error.code,'runtime_unavailable');
mkdirSync(copiedRuntime,{recursive:true});
const copiedPointer = join(copiedRuntime,'current.json');
writeFileSync(copiedPointer,'{invalid');
assert.equal(cli(['--run-action',project.call_path],1,copiedArgs).error.code,'runtime_unavailable');
assert.equal(cli(['--run-action',project.call_path],1,nestedArgs).error.code,'runtime_unavailable');
unlinkSync(copiedPointer);
const override=cli(['--install-script-runtime',development.archive,'--sha256',development.sha256],0,copiedArgs);
assert.equal(override.status,'installed');
const validSourcePointer=readFileSync(pointer,'utf8');
try {
  writeFileSync(pointer,'{invalid');
  assert.equal(cli(['--run-action',project.call_path],0,copiedArgs).output.text,'copied profile');
  assert.equal(cli(['--run-action',project.call_path],0,nestedArgs).output.text,'nested copy');
} finally { writeFileSync(pointer,validSourcePointer); }
checks.push('copied_profile_reuses_host_with_own_code_and_explicit_runtime_override');
checks.push('nested_copy_runs_own_code_with_nearest_managed_host');

const next = variant('next',folder=>{
  const data=Buffer.from('本机更新安装验证');
  writeFileSync(join(folder,'README.txt'),data);
  updateManifest(folder,m=>Object.assign(m.files.find(f=>f.path==='README.txt'),{size:data.length,sha256:hash(data)}));
});
assert.equal(install(next).status,'installed');
assert.notEqual(JSON.parse(readFileSync(pointer,'utf8')).archive_sha256,first.archive_sha256);
assert.equal(hash(readFileSync(firstHost)),hash(originalHost));
assert.equal(cli(['--run-action',project.call_path]).output.text,'OmniTrigger');
checks.push('verified_new_package_switches_pointer_and_preserves_old_package');
assert.equal(readFileSync(config,'utf8'),'[]');
const result = {passed:true,root,checks,personalBindingsModified:false,engine:development.engine,archive_sha256:development.sha256};
writeFileSync(join(root,'result.json'),JSON.stringify(result,null,2));
console.log(JSON.stringify(result));
