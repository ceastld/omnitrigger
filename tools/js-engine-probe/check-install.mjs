// 本机和 CI 的真实安装验收；只使用仓库 .tmp 下的新配置和无界面 probe，不操作输入或剪贴板。
import assert from 'node:assert/strict';
import {execFile, spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, unlinkSync, writeFileSync} from 'node:fs';
import {basename, dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {promisify} from 'node:util';
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
function cli(args, code=0, profileArgs=['--config',config], environment=env) {
  const result = spawnSync(app, [...profileArgs,...args], {encoding:'utf8',windowsHide:true,timeout:120000,maxBuffer:1024*1024,env:environment});
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
  if ($env:TEST_EXTRA) {
    $entry=$zip.CreateEntry($env:TEST_EXTRA)
    if (-not $env:TEST_EXTRA.EndsWith('/')) { $stream=$entry.Open(); $stream.WriteByte(0); $stream.Dispose() }
  }
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
const runtimeRoot = dirname(pointer);
const stages = () => readdirSync(runtimeRoot).filter(name => name.startsWith('.install-')).sort();
const earlier = join(runtimeRoot,'.install-earlier');
mkdirSync(earlier);
writeFileSync(join(earlier,'note.txt'),'previous failure');
const formattedPointer = JSON.stringify(first,null,2) + '\n';
writeFileSync(pointer,formattedPointer);
const repeated = install(development.archive);
assert.equal(repeated.status,'installed');
assert.equal(repeated.cleanup_warning,undefined);
assert.deepEqual(stages(),['.install-earlier']);
assert.equal(readFileSync(join(earlier,'note.txt'),'utf8'),'previous failure');
assert.equal(readFileSync(pointer,'utf8'),formattedPointer);
writeFileSync(pointer,'{');
assert.equal(install(development.archive).status,'installed');
assert.equal(readFileSync(pointer,'utf8'),firstPointer);
checks.push('same_package_reinstall_preserves_valid_pointer_and_restores_invalid_pointer');

for (const args of [
  ['--install-script-runtime',development.archive],
  ['--sha256',development.sha256,'--create-script-action','forbidden'],
  ['--install-script-runtime',development.archive,'--sha256',development.sha256,'--check-system-controls'],
]) assert.equal(cli(args,1).error.code,'invalid_cli_arguments');
assert(!existsSync(join(root,'actions/forbidden')));
checks.push('invalid_CLI_rejected_before_side_effects');

const profileRoot = join(root, 'action-profiles');
const newTest = ['--data-root',profileRoot,'--test-workspace'];
for (const args of [
  ['--install-script-runtime',development.archive],
  ['--create-script-action'],
  ['--run-action',''],
  ['--with-action-state=true'],
  ['--list-package-actions','--with-action-state'],
  ['--stream-action-events'],
  ['--target-context','target.json'],
  ['--check-action'],
  ['--list-package-actions','--create-script-action','forbidden'],
]) {
  const {error} = cli(args,1,newTest);
  assert.equal(error.code,'invalid_cli_arguments');
  assert.equal(error.effect_state,'not_started');
  assert(!existsSync(profileRoot));
}

const selected = ['--data-root',profileRoot,'--profile','pending'];
const {directory} = cli(['--profile-create','pending','--seed','empty'],0,['--data-root',profileRoot]);
const staged = directory + '.creating';
renameSync(directory,staged);
const identity = readFileSync(join(staged,'profile.json'));
const catalogBefore = readFileSync(join(profileRoot,'profiles.json'));
for (const profileArgs of [selected, ['--config',join(directory,'bindings.json')]]) {
  const {error} = cli(['--run-action','call.json','--target-context'],1,profileArgs);
  assert.equal(error.code,'invalid_cli_arguments');
  assert.equal(error.effect_state,'not_started');
  assert(!existsSync(directory));
  assert.deepEqual(readFileSync(join(staged,'profile.json')),identity);
  assert.deepEqual(readFileSync(join(profileRoot,'profiles.json')),catalogBefore);
}
const recovered = cli(['--create-script-action','profile-only'],0,selected);
assert(existsSync(recovered.project.entry_path));
assert(existsSync(join(directory,'actions/profile-only/action.json')));
assert(!existsSync(staged));
assert(!existsSync(join(profileRoot,'actions/profile-only')));
checks.push('invalid_action_arguments_preserve_new_and_pending_profiles');

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

const legacyHost=join(root,'runtimes/script/host.exe');
cpSync(firstHost,legacyHost,{errorOnExist:true});
try {
  writeFileSync(pointer,JSON.stringify({...first,manifest_sha256:'0'.repeat(64)}));
  const failure=cli(['--run-action',project.call_path],1);
  assert.equal(failure.error.code,'runtime_unavailable');
} finally { writeFileSync(pointer,firstPointer); unlinkSync(legacyHost); }
checks.push('invalid_managed_pointer_does_not_fall_back');

const jsonPackage = join(root,'actions','json-text');
cpSync(join(repo,'examples/actions/json-text'),jsonPackage,{recursive:true,errorOnExist:true});
const nodeArgs = ['--run-action',join(jsonPackage,'call.json')];
const copy = cli(['--profile-create','runtime-copy','--seed','copy:default'],0,['--data-root',root]);
const copiedArgs = ['--data-root',root,'--profile','runtime-copy'];
assert(!existsSync(join(copy.directory,'runtimes')));
const runtimeProfiles = [['--config',config],copiedArgs];
const changed = Buffer.from(originalHost);
changed[changed.length-1] ^= 1;
try {
  writeFileSync(firstHost,changed);
  for (const profileArgs of runtimeProfiles) {
    const checked = cli(['--check-action',project.call_path],1,profileArgs);
    assert.equal(checked.error.code,'runtime_integrity');
    assert.equal(checked.error.effect_state,'not_started');
    const catalog = cli(['--list-package-actions'],0,profileArgs);
    const row = catalog.actions.find(({call}) => call.package_id === project.call.package_id && call.action_id === project.call.action_id);
    assert.equal(row.reference_error.code,'runtime_integrity');
  }
  const failure = cli(['--run-action',project.call_path],1);
  assert.equal(failure.status,'failed');
  assert.equal(failure.error.code,'runtime_integrity');
  assert.equal(failure.error.effect_state,'not_started');
  const directory = dirname(firstHost);
  const note = join(directory,'personal-note.txt');
  writeFileSync(note,'keep');
  assert.match(install(development.archive,1).error.message,/清单外文件/);
  assert.equal(readFileSync(note,'utf8'),'keep');
  assert.deepEqual(readFileSync(firstHost),changed);
  assert.equal(readFileSync(pointer,'utf8'),firstPointer);
  unlinkSync(note);
  const sdk = join(directory,'sdk','api.d.ts');
  const sdkBefore = hash(readFileSync(sdk));
  unlinkSync(sdk);
  writeFileSync(join(directory,'manifest.json'),'{');
  const repaired = install(development.archive);
  assert.equal(repaired.repaired_files,3);
  assert.equal(repaired.cleanup_warning,undefined);
  assert.equal(readFileSync(pointer,'utf8'),firstPointer);
  assert.deepEqual(readFileSync(firstHost),originalHost);
  assert.equal(hash(readFileSync(sdk)),sdkBefore);
} finally { writeFileSync(firstHost,originalHost); }
for (const profileArgs of runtimeProfiles) {
  assert.equal(cli(['--check-action',project.call_path],0,profileArgs).valid,true);
}
checks.push('same_size_installed_image_tampering_rejected_before_execution');
checks.push('catalog_and_check_report_damaged_managed_host_in_daily_and_copy');
checks.push('same_archive_repairs_missing_and_damaged_managed_files_without_overwriting_unknown_files');
assert.equal(cli(['--run-action',project.call_path],0,copiedArgs).output.text,'OmniTrigger');
const copiedPackage = join(copy.directory,'actions','installed-script');
const copiedManifest = JSON.parse(readFileSync(join(copiedPackage,'action.json'),'utf8'));
const copiedEntry = join(copiedPackage,copiedManifest.actions[0].entry);
writeFileSync(copiedEntry,'export function main() { return {text:"copied profile"}; }');
assert.equal(cli(['--run-action',project.call_path],0,copiedArgs).output.text,'copied profile');
assert.equal(cli(['--run-action',project.call_path]).output.text,'OmniTrigger');
const cleared = {...env, OMNITRIGGER_NODE:'', OMNITRIGGER_SCRIPT_HOST:''};
const missing = {...env, OMNITRIGGER_SCRIPT_HOST:join(root,'missing-script-host.exe')};
for (const [profileArgs,text] of [[['--config',config],'OmniTrigger'],[copiedArgs,'copied profile']]) {
  const scriptArgs = ['--run-action',project.call_path];
  assert.equal(cli(scriptArgs,0,profileArgs,cleared).output.text,text);
  assert.equal(cli(nodeArgs,0,profileArgs,cleared).output.status,'inspected');
  const scriptError = cli(scriptArgs,1,profileArgs,missing).error;
  assert.equal(scriptError.code, 'runtime_unavailable');
  assert.match(scriptError.message, /OMNITRIGGER_SCRIPT_HOST/);
  const nodeError = cli(nodeArgs,1,profileArgs,missing).error;
  assert.equal(nodeError.code, 'runtime_unavailable');
  assert.match(nodeError.message, /Node\.js/);
}
checks.push('empty_host_paths_restore_default_runtimes_in_daily_and_copied_profiles');
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
function checkUnavailableScript(args, reason) {
  const catalog = cli(['--list-package-actions'], 0, args);
  const row = catalog.actions.find(({call}) => call.package_id === project.call.package_id && call.action_id === project.call.action_id);
  assert.equal(row.reference_error.code, 'runtime_unavailable');
  assert.match(row.reference_error.message, reason);
  assert(catalog.diagnostics.includes(row.reference_error.message));
  for (const flag of ['--check-action', '--run-action']) {
    const failure = cli([flag, project.call_path], 1, args);
    assert.equal(failure.error.code, 'runtime_unavailable');
    assert.equal(failure.error.message, row.reference_error.message);
    assert.equal(failure.error.effect_state, 'not_started');
  }
}
checkUnavailableScript(['--data-root',root,'--profile',bareCopy.id], /安装宿主包/);
mkdirSync(copiedRuntime,{recursive:true});
const copiedPointer = join(copiedRuntime,'current.json');
writeFileSync(copiedPointer,'{invalid');
checkUnavailableScript(copiedArgs, /版本记录格式有误/);
checkUnavailableScript(nestedArgs, /版本记录格式有误/);
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

// 重置应恢复来源的数据，并保留本档明确安装的解释器。
const resetCopy = () => cli(['--profile-reset','runtime-copy'],0,['--data-root',root]);
const activeCopyPointer = readFileSync(copiedPointer,'utf8');
try {
  writeFileSync(pointer,'{invalid');
  const restored = resetCopy();
  assert.equal(restored.runtime_warning,undefined);
  assert.equal(readFileSync(copiedPointer,'utf8'),activeCopyPointer);
  assert.equal(cli(['--run-action',project.call_path],0,copiedArgs).output.text,'OmniTrigger');
  assert.match(readFileSync(join(restored.backup,'actions','installed-script',copiedManifest.actions[0].entry),'utf8'),/copied profile/);

  unlinkSync(override.program);
  const damaged = resetCopy();
  assert.match(damaged.runtime_warning,/脚本宿主/);
  assert.equal(existsSync(copiedPointer),false);
  assert.equal(readFileSync(join(damaged.backup,'runtimes','script','current.json'),'utf8'),activeCopyPointer);
  assert.equal(cli(['--run-action',project.call_path],1,copiedArgs).error.code,'runtime_unavailable');
} finally { writeFileSync(pointer,validSourcePointer); }
checks.push('reset_preserves_own_verified_runtime_and_reports_a_damaged_install');

const next = variant('next',folder=>{
  const data=Buffer.from('本机更新安装验证');
  writeFileSync(join(folder,'README.txt'),data);
  updateManifest(folder,m=>Object.assign(m.files.find(f=>f.path==='README.txt'),{size:data.length,sha256:hash(data)}));
},'licenses/empty/subdirectory/');
const beforeConcurrent = stages();
const runFile = promisify(execFile);
const args = ['--config',config,'--install-script-runtime',next,'--sha256',hash(readFileSync(next))];
const installNext = () => runFile(app,args,{encoding:'utf8',windowsHide:true,env,timeout:120000});
const installed = await Promise.all([installNext(),installNext()]);
for (const result of installed) {
  const receipt = JSON.parse(result.stdout);
  assert.equal(receipt.status,'installed');
  assert.equal(receipt.cleanup_warning,undefined);
  assert(existsSync(join(dirname(receipt.program),'licenses','empty','subdirectory')));
}
assert.deepEqual(stages(),beforeConcurrent);
checks.push('concurrent_same_archive_installs_clear_only_their_staging');
assert.notEqual(JSON.parse(readFileSync(pointer,'utf8')).archive_sha256,first.archive_sha256);
assert.equal(hash(readFileSync(firstHost)),hash(originalHost));
assert.equal(cli(['--run-action',project.call_path]).output.text,'OmniTrigger');
checks.push('verified_new_package_switches_pointer_and_preserves_old_package');

const activeAfterSwitch = readFileSync(pointer,'utf8');
const runtimeFiles = () => readdirSync(runtimeRoot,{withFileTypes:true}).filter(entry=>entry.isFile()).map(entry=>entry.name).sort();
const beforeFailure = runtimeFiles();
chmodSync(pointer,0o444);
try {
  const refused = install(development.archive,1);
  assert.equal(refused.error.code,'runtime_install_failed');
  assert(refused.error.message.includes(pointer),refused.error.message);
  assert.equal(readFileSync(pointer,'utf8'),activeAfterSwitch);
  assert.deepEqual(runtimeFiles(),beforeFailure);
  assert.equal(cli(['--run-action',project.call_path]).output.text,'OmniTrigger');
} finally { chmodSync(pointer,0o666); }
assert.equal(install(development.archive).status,'installed');
assert.equal(readFileSync(pointer,'utf8'),firstPointer);
checks.push('blocked_switch_preserves_active_actions_and_retry_activates_the_requested_package');

assert.equal(readFileSync(config,'utf8'),'[]');
const result = {passed:true,root,checks,personalBindingsModified:false,engine:development.engine,archive_sha256:development.sha256};
writeFileSync(join(root,'result.json'),JSON.stringify(result,null,2));
console.log(JSON.stringify(result));
