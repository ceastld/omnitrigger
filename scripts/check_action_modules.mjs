// 独立离线工具：默认仅解析 ESM 静态依赖，不改变 check-action 的含义。
import {readFileSync, writeFileSync, lstatSync} from 'node:fs';
import {resolve, dirname, relative, extname, sep, isAbsolute} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
import {spawnSync} from 'node:child_process';
import {SourceTextModule} from 'node:vm';

function argumentsOf(argv) {
  const result = {trusted:false};
  for (let index = 0; index < argv.length; index++) {
    if (argv[index] === '--import-trusted-project') result.trusted = true;
    else if (['--snapshot','--out','--python'].includes(argv[index])) {
      const name = argv[index].slice(2);
      if (result[name] || !argv[index + 1] || argv[index + 1].startsWith('--')) throw Error('参数无效');
      result[name] = argv[++index];
    } else throw Error('参数无效');
  }
  if (!result.snapshot || !result.out) throw Error('需要 --snapshot 和全新 --out');
  return result;
}

function contained(root, value) {
  const suffix = relative(root, value);
  if (isAbsolute(suffix) || suffix === '..' || suffix.startsWith('..' + sep) || resolve(root, suffix) !== value) throw Error('模块路径越界');
  return value;
}

const options = argumentsOf(process.argv.slice(2));
const reportPath = resolve(options.snapshot);
const snapshotRoot = dirname(reportPath);
const outputPath = resolve(options.out);
if (dirname(outputPath) !== snapshotRoot) throw Error('检查报告只能独占新建在已验证快照根目录');
try { lstatSync(outputPath); throw Error('检查报告已存在，不执行导入也不覆盖'); }
catch (error) { if (error.code !== 'ENOENT') throw error; }
const checker = fileURLToPath(new URL('./snapshot_action_candidates.py', import.meta.url));
const trustedRoot = fileURLToPath(new URL('../examples/actions/', import.meta.url));
function verify(trusted = false) {
  const result = spawnSync(options.python || 'python', ['-B', checker, '--verify-snapshot', reportPath,
    ...(trusted ? ['--trusted-root', resolve(trustedRoot)] : [])],
  {encoding:'utf8', timeout:30_000, maxBuffer:1024 * 1024, windowsHide:true});
  if (result.status !== 0 || result.error) throw Error('快照完整性或可信来源验证失败；未导入任何模块');
}
verify();
const snapshot = JSON.parse(readFileSync(reportPath, 'utf8'));
const report = {version:1, static_graph_passed:false, static_only:!options.trusted,
  trusted_node_import_requested:options.trusted, trusted_initializers_evaluated:false,
  entry_functions_called:false, actions_executed:false, activated:false,
  dynamic_imports_and_function_bodies_verified:false, packages:[], imports:[]};
let failure = false;

for (const item of snapshot.packages) {
  const packageRoot = contained(snapshotRoot, resolve(snapshotRoot, item.snapshot_root));
  const files = new Set(item.files.map(row => row.path));
  const edges = [];
  const seen = new Set();
  const errors = [];
  function walk(name) {
    if (seen.has(name)) return;
    seen.add(name);
    if (!files.has(name) || !['.mjs','.js'].includes(extname(name))) {
      errors.push({file:name, code:'missing_or_unsupported_module'}); return;
    }
    let parsed;
    try {
      parsed = new SourceTextModule(readFileSync(contained(packageRoot, resolve(packageRoot, name)), 'utf8'));
    } catch (_) { errors.push({file:name, code:'module_parse_failed'}); return; }
    // dependencySpecifiers 仅含静态 import/export-from，不评估任何顶层代码。
    for (const specifier of parsed.dependencySpecifiers) {
      if (!specifier.startsWith('./') && !specifier.startsWith('../') || /[\\\0%?#:]/.test(specifier)) {
        errors.push({file:name, code:'non_relative_module'}); continue;
      }
      let target;
      try { target = contained(packageRoot, resolve(packageRoot, dirname(name), specifier)); }
      catch (_) { errors.push({file:name, code:'module_outside_package'}); continue; }
      const child = relative(packageRoot, target).split(sep).join('/');
      edges.push({from:name, to:child});
      walk(child);
    }
  }
  for (const action of item.manifest.actions) walk(action.entry);
  failure ||= errors.length > 0;
  report.packages.push({id:item.id, modules:[...seen].sort(), edges, errors,
    static_imports_only:true, module_initializers_evaluated:false});
}
report.static_graph_passed = !failure;
if (options.trusted && !failure) {
  // 只有与脚本固定的本项目 examples/actions 完整文件哈希一致才可显式执行初始化。
  // 这不是用户动作验证接口，既不接收 ActionContext，也不调用导出的入口函数。
  verify(true);
  for (const item of snapshot.packages) {
    for (const action of item.manifest.actions.filter(row => row.runtime === 'node')) {
      const entry = contained(snapshotRoot, resolve(snapshotRoot, item.snapshot_root, action.entry));
      const code = `const module = await import(process.argv[1]); if (typeof module.default !== 'function') process.exit(3);`;
      const child = spawnSync(process.execPath, ['--input-type=module','--eval',code,pathToFileURL(entry).href],
        {encoding:'utf8',timeout:5000,maxBuffer:64 * 1024,windowsHide:true});
      const passed = child.status === 0 && !child.error;
      report.imports.push({package_id:item.id, action_id:action.id, entry:action.entry,
        passed, trusted_project_only:true, initializer_evaluated:true, entry_called:false,
        ...(passed ? {} : {code:'trusted_import_failed'})});
      report.trusted_initializers_evaluated = true;
      failure ||= !passed;
    }
  }
}
report.passed = !failure;
// 独占新建报告，不覆盖先前静态或导入证据。
writeFileSync(outputPath, JSON.stringify(report, null, 2) + '\n', {flag:'wx'});
process.stdout.write(JSON.stringify({passed:report.passed, packages:report.packages.length,
  trusted_imports:report.imports.length, entry_functions_called:false}) + '\n');
if (failure) process.exitCode = 1;
