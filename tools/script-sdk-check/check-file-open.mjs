// 实际文件分支验收。只有显式提供并复制到新目录的无界面 probe 会经真实 ctx.open 启动。
// CLI: node check-file-open.mjs <app.exe> <file-open-probe.exe> <script-host.exe> [另一个宿主.exe]
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {
  constants, closeSync, copyFileSync, fstatSync, fsyncSync, ftruncateSync, futimesSync,
  lstatSync, mkdirSync, mkdtempSync, openSync, readFileSync, readSync, realpathSync,
  writeFileSync, writeSync,
} from 'node:fs';
import {basename, dirname, isAbsolute, join, relative, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const temporaryRoot = join(repo, '.tmp');
const recipePath = join(repo, 'examples/actions/common-search/open-or-search.js');
const MAX_FILE = 64 * 1024 * 1024;
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const waitBuffer = new Int32Array(new SharedArrayBuffer(4));

function inside(parent, child) {
  const part = relative(resolve(parent), resolve(child));
  return part !== '' && part !== '..' && !part.startsWith('..\\') && !part.startsWith('../') && !isAbsolute(part);
}
function localAbsolute(path) {
  assert.equal(typeof path, 'string', '验收路径必须是字符串');
  assert(/^[A-Za-z]:[\\/]/.test(path) && isAbsolute(path), '验收只接受普通盘符绝对路径');
  assert(!/[\u0000-\u001f\u007f]/u.test(path), '验收路径不能包含控制字符');
  const parts = path.slice(3).split(/[\\/]/u);
  assert(!parts.some(part => /^(?:OneDrive|iCloud)/iu.test(part) || /^Dropbox$/iu.test(part)), '验收不访问云盘目录');
  assert(!parts.some(part => part === '.' || part === '..' || part.includes(':')), '验收路径不能含跳转或备用数据流');
  return resolve(path);
}

// lstat 不跟随链接；随后用 Windows 原生属性排除非符号链接的 reparse 和召回属性。
// 只查询列出的路径及祖先，不枚举目录、不读取内容、不动态编译代码。
function plainPaths(paths) {
  const all = new Set();
  for (const input of paths) {
    const path = localAbsolute(input);
    const chain = [];
    for (let current = path;; current = dirname(current)) {
      chain.push(current);
      if (dirname(current) === current) break;
    }
    for (const current of chain.reverse()) {
      const metadata = lstatSync(current);
      assert(!metadata.isSymbolicLink(), '验收路径及祖先不能是链接或联接点');
      if (current !== path) assert(metadata.isDirectory(), '验收路径祖先必须是目录');
      all.add(current);
    }
  }
  const powershell = join(process.env.SystemRoot ?? 'C:\\Windows', 'System32/WindowsPowerShell/v1.0/powershell.exe');
  const check = spawnSync(powershell, ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', `
$ErrorActionPreference='Stop'
try {
  [Console]::InputEncoding=[System.Text.UTF8Encoding]::new($false)
  [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false)
  $OutputEncoding=[Console]::OutputEncoding
  $paths=ConvertFrom-Json ([Console]::In.ReadToEnd())
  foreach ($path in $paths) {
    $attributes=[int][IO.File]::GetAttributes($path)
    if (($attributes -band (0x400 -bor 0x1000 -bor 0x40000 -bor 0x400000)) -ne 0) { throw 'blocked' }
  }
} catch { [Console]::Error.Write('验收路径包含重解析点、占位属性或不可用的元数据。'); exit 1 }
`], {input: JSON.stringify([...all]), encoding: 'utf8', windowsHide: true, timeout: 10000, maxBuffer: 4096});
  assert.ifError(check.error);
  assert.equal(check.status, 0, check.stderr || '无法检查 Windows 文件属性');
}

function plainFile(path, maximum = MAX_FILE, singleLink = false) {
  plainPaths([path]);
  const metadata = lstatSync(path);
  assert(metadata.isFile(), '验收文件必须是普通文件');
  if (singleLink) assert.equal(metadata.nlink, 1, '受保护文件和 marker 不能有硬链接');
  assert(metadata.size <= maximum, '验收文件超过大小上限');
  return metadata;
}
function sameFile(left, right) { return left.dev === right.dev && left.ino === right.ino; }
function readDescriptor(fd) {
  const size = fstatSync(fd).size;
  assert(size <= MAX_FILE, '保护文件超过大小上限');
  const bytes = Buffer.alloc(size);
  let offset = 0;
  while (offset < size) {
    const count = readSync(fd, bytes, offset, size - offset, offset);
    assert(count > 0, '读取保护文件时内容发生变化');
    offset += count;
  }
  return bytes;
}
function protectFile(root, path) {
  assert(inside(root, path), '只能保护本轮隔离目录内的文件');
  const metadata = plainFile(path, 256 * 1024, true);
  const fd = openSync(path, constants.O_RDWR | (constants.O_NOFOLLOW ?? 0));
  try {
    assert(sameFile(metadata, fstatSync(fd)), '打开保护文件前身份发生变化');
    return {root, path, fd, metadata, bytes: readDescriptor(fd)};
  } catch (error) { closeSync(fd); throw error; }
}
function replaceContents(snapshot, bytes) {
  plainPaths([snapshot.root, snapshot.path]);
  assert(inside(realpathSync(snapshot.root), realpathSync(snapshot.path)), '保护文件不能越出隔离目录');
  assert(sameFile(lstatSync(snapshot.path), fstatSync(snapshot.fd)), '保护文件路径身份发生变化');
  assert.equal(fstatSync(snapshot.fd).nlink, 1, '保护文件不能通过硬链接写到其他位置');
  ftruncateSync(snapshot.fd, 0);
  let offset = 0;
  while (offset < bytes.length) {
    const count = writeSync(snapshot.fd, bytes, offset, bytes.length - offset, offset);
    assert(count > 0, '写入保护文件时没有取得进展');
    offset += count;
  }
  fsyncSync(snapshot.fd);
}
function restoreFile(snapshot) {
  try {
    plainPaths([snapshot.path]);
    assert(sameFile(lstatSync(snapshot.path), fstatSync(snapshot.fd)), '验收后保护文件身份发生变化');
    const metadata = fstatSync(snapshot.fd);
    assert.equal(metadata.nlink, 1, '恢复时保护文件不能有硬链接');
    if (metadata.size !== snapshot.bytes.length || !readDescriptor(snapshot.fd).equals(snapshot.bytes)) {
      replaceContents(snapshot, snapshot.bytes);
    }
    assert(readDescriptor(snapshot.fd).equals(snapshot.bytes), '没有恢复保护文件的原始内容');
    futimesSync(snapshot.fd, snapshot.metadata.atime, snapshot.metadata.mtime);
  } finally { closeSync(snapshot.fd); }
}
function absent(path) {
  try { lstatSync(path); return false; }
  catch (error) { if (error.code === 'ENOENT') return true; throw error; }
}

function probeImage(bytes) {
  assert(bytes.length >= 256 && bytes.toString('ascii', 0, 2) === 'MZ', 'probe 不是 PE 可执行文件');
  const pe = bytes.readUInt32LE(0x3c);
  assert(pe >= 0x40 && pe + 24 + 70 <= bytes.length, 'probe PE 头越界');
  assert.equal(bytes.readUInt32LE(pe), 0x00004550, 'probe 缺少 PE 签名');
  const flags = bytes.readUInt16LE(pe + 22);
  assert((flags & 2) !== 0 && (flags & 0x2000) === 0, 'probe 必须是可执行程序，不能是 DLL');
  assert([0x10b, 0x20b].includes(bytes.readUInt16LE(pe + 24)), 'probe PE 格式无效');
  assert.equal(bytes.readUInt16LE(pe + 24 + 68), 2, 'probe 必须使用无控制台的 Windows 子系统');
  assert(bytes.includes(Buffer.from('.opened.json')) && bytes.includes(Buffer.from('"opened":true,"process_id":')), 'probe 缺少约定 marker 特征');
}

function waitMarker(marker, started) {
  const deadline = performance.now() + 10000;
  while (performance.now() < deadline) {
    if (!absent(marker)) {
      const metadata = plainFile(marker, 256, true);
      assert(metadata.birthtimeMs >= started - 2000 && metadata.mtimeMs >= started - 2000, 'marker 不是本轮新建文件');
      let value;
      try { value = JSON.parse(readFileSync(marker, 'utf8')); }
      catch (error) { if (!(error instanceof SyntaxError)) throw error; }
      if (value !== undefined) {
        assert.deepEqual(Object.keys(value).sort(), ['opened', 'process_id']);
        assert.equal(value.opened, true, 'probe 没有确认实际启动');
        assert(Number.isInteger(value.process_id) && value.process_id > 0 && value.process_id <= 0xffffffff && value.process_id !== process.pid, 'probe 进程 ID 无效');
        return value;
      }
    }
    Atomics.wait(waitBuffer, 0, 0, 25);
  }
  assert.fail('没有在 10 秒内观察到 probe 全新启动 marker；动作接纳不等于实际文件启动');
}

/**
 * 同步验收，可直接接入 check-install.mjs 的同步 CLI 适配。
 * root 必须是仓库 .tmp 下本轮新建的隔离工程，含内容为 [] 的 bindings.json；
 * entryPath 必须是其 actions/<包>/ 中清单指定的轻量入口。
 * invoke(callPath) 必须同步执行该隔离配置的真实应用 CLI，返回 ActionOutcome JSON。
 * probeExecutable 必须由本轮 file-open-probe.rs 编译，不能使用个人程序。
 * 本函数不编译，不安装宿主，不覆盖 ctx.open；入口/配置/清单通过原文件句柄恢复。
 */
export function verifyFileOpen({root, entryPath, call, invoke, probeExecutable}) {
  assert.equal(process.platform, 'win32', '真实文件启动验收仅支持 Windows');
  const current = localAbsolute(root);
  const entry = localAbsolute(entryPath);
  const probeSource = localAbsolute(probeExecutable);
  assert(inside(temporaryRoot, current), '隔离 root 必须位于仓库 .tmp 的子目录');
  assert(inside(join(current, 'actions'), entry), '入口必须属于本轮隔离 actions 工程');
  assert(inside(repo, probeSource), 'probe 必须来自本仓库显式构建路径');
  assert.equal(typeof invoke, 'function', '缺少同步真实应用 invoke 适配');
  plainPaths([current, entry, probeSource, recipePath]);
  assert(inside(realpathSync(temporaryRoot), realpathSync(current)), '隔离 root 不能通过链接越界');
  assert(inside(realpathSync(current), realpathSync(entry)), '入口不能通过链接越界');
  const packageRoot = dirname(entry);
  assert.equal(dirname(packageRoot), join(current, 'actions'), '入口需位于本轮直接子包，不能重用其他配置');
  const manifestPath = join(packageRoot, 'action.json');
  const config = join(current, 'bindings.json');
  plainFile(manifestPath, 256 * 1024);
  plainFile(config, 128 * 1024);
  plainFile(recipePath, 256 * 1024);
  plainFile(probeSource);
  const recipe = readFileSync(recipePath, 'utf8');
  const image = readFileSync(probeSource);
  probeImage(image);
  assert.equal(readFileSync(config, 'utf8').trim(), '[]', '只允许本轮空配置，不能使用个人绑定');
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  assert.equal(manifest.schema_version, 1);
  assert(call && call.package_id === manifest.id, '调用必须属于本轮动作包');
  const definitions = manifest.actions.filter(action => action.id === call.action_id);
  assert.equal(definitions.length, 1, '动作身份必须唯一');
  const definition = definitions[0];
  assert.equal(definition.runtime, 'script', '验收需要真实轻量脚本宿主');
  assert.equal(definition.version, call.action_version, '动作契约版本不匹配');
  assert.equal(resolve(packageRoot, definition.entry), entry, '清单入口与本轮保护入口不同');

  const snapshots = [];
  let result;
  let failure;
  try {
    snapshots.push(protectFile(current, entry));
    snapshots.push(protectFile(current, config));
    snapshots.push(protectFile(current, manifestPath));
    const fixture = mkdtempSync(join(current, 'file-open-'));
    plainPaths([fixture]);
    const executable = join(fixture, '文件启动 😀.exe');
    const marker = join(fixture, '.opened.json');
    assert(inside(current, executable) && inside(current, marker));
    assert(absent(executable) && absent(marker), '本轮 probe 目标或 marker 已存在');
    copyFileSync(probeSource, executable, constants.COPYFILE_EXCL);
    plainFile(executable);
    assert.equal(hash(readFileSync(executable)), hash(image), '复制后的 probe 与显式输入不一致');

    // 执行实际 recipe 源码；ctx.open 从真实 ctx 原样保留，绝不收集/替换打开请求。
    // 只在真实 stat 外加精确目标与 kind=file 检查。若文件消失，直接失败而非打开搜索浏览器。
    const source = `function runActualRecipe(ctx) {\n${recipe}\nreturn main(ctx);\n}
export function main(ctx) {
  const expected = ${JSON.stringify(executable)};
  if (ctx.input.source !== 'text' || ctx.input.text !== expected) throw new Error('验收输入不是本轮 probe');
  const checkedFs = {...ctx.fs, stat(path) {
    if (path !== expected) throw new Error('元数据目标越出本轮 probe');
    const metadata = ctx.fs.stat(path);
    if (!metadata || metadata.kind !== 'file') throw new Error('本轮 probe 不再是普通文件，禁止进入搜索分支');
    return metadata;
  }};
  return runActualRecipe({...ctx, fs:checkedFs});
}`;
    replaceContents(snapshots[0], Buffer.from(source));
    const request = {...call, input:{source:'text', text:executable}};
    const callPath = join(fixture, 'call.json');
    plainPaths([dirname(callPath)]);
    writeFileSync(callPath, JSON.stringify(request), {flag:'wx'});
    assert(absent(marker), '调用前 marker 必须不存在');
    const started = Date.now();
    const response = invoke(callPath);
    assert(response && typeof response === 'object' && !Array.isArray(response), 'invoke 没有返回真实应用 JSON 对象');
    assert(!response?.then, 'invoke 必须同步返回真实应用 JSON');
    assert.equal(response.status, 'succeeded', JSON.stringify(response));
    assert.deepEqual(response.output, {submitted:true, source:'text', operation:'open-file'});
    const opened = waitMarker(marker, started);
    plainPaths([entry, config, manifestPath, executable, marker, recipePath]);
    assert(readDescriptor(snapshots[0].fd).equals(Buffer.from(source)), '运行中验收入口被意外修改');
    assert(readDescriptor(snapshots[1].fd).equals(snapshots[1].bytes), '运行中隔离配置被意外修改');
    assert(readDescriptor(snapshots[2].fd).equals(snapshots[2].bytes), '运行中清单被意外修改');
    assert.equal(hash(readFileSync(executable)), hash(image), '运行后 probe 文件身份内容变化');
    assert.equal(readFileSync(recipePath, 'utf8'), recipe, '实际示例源码被意外修改');
    result = {
      passed:true, actualFileBranch:true, freshMarkerObserved:true, process_id:opened.process_id,
      noOpenStub:true, nonFileSearchPrevented:true, probe_sha256:hash(image), recipe_sha256:hash(recipe),
      markerPath:marker, executablePath:executable, entryRestored:true, configPreserved:true,
      personalBindingsModified:false,
    };
  } catch (error) { failure = error; }
  const restoreErrors = [];
  for (const snapshot of snapshots.reverse()) {
    try { restoreFile(snapshot); } catch (error) { restoreErrors.push(error); }
  }
  if (restoreErrors.length) throw new AggregateError([...(failure ? [failure] : []), ...restoreErrors], '文件启动验收或原始入口/配置恢复失败');
  if (failure) throw failure;
  return result;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  assert.equal(process.platform, 'win32', '此检查需要 Windows 应用和显式轻量宿主');
  assert(process.argv.length >= 5, '用法：node check-file-open.mjs <app.exe> <file-open-probe.exe> <script-host.exe> [另一个宿主.exe]');
  const sourceApp = localAbsolute(resolve(process.argv[2]));
  const probeExecutable = localAbsolute(resolve(process.argv[3]));
  const hosts = process.argv.slice(4).map(path => localAbsolute(resolve(path)));
  plainFile(sourceApp);
  plainFile(probeExecutable);
  for (const host of hosts) plainFile(host);
  plainPaths([repo]);
  if (absent(temporaryRoot)) mkdirSync(temporaryRoot);
  plainPaths([temporaryRoot]);
  const root = mkdtempSync(join(temporaryRoot, 'file-open-check-'));
  const app = join(root, 'application.exe');
  copyFileSync(sourceApp, app, constants.COPYFILE_EXCL);
  const checks = [];
  for (const [index, host] of hosts.entries()) {
    const current = join(root, `script-${index + 1}`);
    const packageRoot = join(current, 'actions', 'file-open-check');
    mkdirSync(packageRoot, {recursive:true});
    const config = join(current, 'bindings.json');
    const entryPath = join(packageRoot, 'main.mjs');
    writeFileSync(config, '[]', {flag:'wx'});
    writeFileSync(entryPath, 'export function main() { return null; }\n', {flag:'wx'});
    writeFileSync(join(packageRoot, 'action.json'), JSON.stringify({schema_version:1,id:'file-open-check',name:'合成文件启动验收',version:'1.0.0',actions:[{id:'run',title:'启动本轮probe',version:1,runtime:'script',entry:'main.mjs'}]}), {flag:'wx'});
    // 必须使用显式宿主，不读取个人已安装宿主；同时让 Node 路径明确不可用。
    const env = {...process.env, OMNITRIGGER_SCRIPT_HOST:host, OMNITRIGGER_NODE:join(current, 'missing-node.exe')};
    const result = verifyFileOpen({root:current, entryPath, call:{package_id:'file-open-check',action_id:'run',action_version:1,input:{}}, probeExecutable, invoke:callPath => {
      assert(inside(current, callPath), '调用文件越出本轮隔离配置');
      // 与安装验收保持同一外层上限。系统打开接口首次接纳新 EXE 可能超过 30 秒；
      // 宿主自身的 60 秒执行期限保持不变，仍须同时收到动作成功和新进程回执。
      const child = spawnSync(app, ['--config',config,'--run-action',callPath], {encoding:'utf8',windowsHide:true,timeout:120000,maxBuffer:1024*1024,env});
      assert.ifError(child.error);
      assert.equal(child.status, 0, child.stderr || child.stdout);
      return JSON.parse(child.stdout);
    }});
    checks.push({...result, mode:`script-${index + 1}`, hostName:basename(host)});
  }
  const result = {passed:true, root, checks, explicitScriptHosts:true, personalBindingsModified:false};
  writeFileSync(join(root, 'result.json'), JSON.stringify(result, null, 2), {flag:'wx'});
  console.log(JSON.stringify(result));
}
