// 清单与无 Hook CLI 验收；互斥执行由 ActionService 的真实运行时集成测试覆盖。
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const application = resolve(process.argv[2] ?? 'target/debug/omnitrigger.exe');
mkdirSync(join(repo, '.tmp'), {recursive:true});
const root = mkdtempSync(join(repo, '.tmp/single-instance-cli-'));
const config = join(root, 'bindings.json');
writeFileSync(config, '[]', {flag:'wx'});
const folder = join(root, 'actions', 'policy-check');
mkdirSync(folder, {recursive:true});
const marker = join(root, 'must-not-execute.txt');
const entry = `await ctx.fs.writeText(${JSON.stringify(marker)}, 'executed'); return null;\n`;
writeFileSync(join(folder, 'main.js'), entry, {flag:'wx'});
const definition = id => ({id, title:id, version:1, runtime:'node', entry:'main.js'});
const manifest = {
  schema_version:1, id:'policy-check', name:'单实例清单验收', version:'1.0.0',
  actions:[definition('default'), {...definition('explicit-false'), single_instance:false},
    {...definition('exclusive'), single_instance:true}],
};
writeFileSync(join(folder, 'action.json'), JSON.stringify(manifest), {flag:'wx'});
const env = {...process.env, OMNITRIGGER_NODE:process.execPath};
delete env.OMNITRIGGER_SCRIPT_HOST;
function cli(args, code = 0) {
  const child = spawnSync(application, ['--config', config, ...args], {
    windowsHide:true, encoding:'utf8', timeout:30000, maxBuffer:1024*1024, env,
  });
  assert.ifError(child.error);
  assert.equal(child.status, code, child.stderr || child.stdout);
  assert.equal(existsSync(marker), false, '目录或引用校验执行了脚本');
  assert.equal(readFileSync(config, 'utf8'), '[]');
  assert.equal(readFileSync(join(folder, 'main.js'), 'utf8'), entry);
  return JSON.parse(code === 0 ? child.stdout : child.stderr);
}
const listed = cli(['--list-package-actions']);
assert.equal(listed.schema_version, 1);
assert.deepEqual(listed.diagnostics, []);
assert.equal(listed.actions.length, 3);
for (const action of listed.actions) {
  assert.equal(action.reference_error, null);
  if (action.call.action_id === 'exclusive') assert.equal(action.single_instance, true);
  else assert.equal(Object.hasOwn(action, 'single_instance'), false, '默认目录 JSON 不应新增 false 字段');
  const call = join(root, `${action.call.action_id}.json`);
  writeFileSync(call, JSON.stringify(action.call), {flag:'wx'});
  const checked = cli(['--check-action', call]);
  assert.equal(checked.valid, true);
}
const invalidFolder = join(root, 'actions', 'invalid-policy');
mkdirSync(invalidFolder);
writeFileSync(join(invalidFolder, 'main.js'), entry, {flag:'wx'});
for (const value of [null, 0, 1, 'true', [], {}]) {
  writeFileSync(join(invalidFolder, 'action.json'), JSON.stringify({
    schema_version:1, id:'invalid-policy', name:'无效策略验收', version:'1.0.0',
    actions:[{...definition('run'), single_instance:value}],
  }));
  const result = cli(['--list-package-actions']);
  assert.equal(result.actions.length, 3);
  assert(result.actions.every(action => action.call.package_id === 'policy-check'));
  assert(result.diagnostics.length > 0, '错误类型不能被静默转成默认策略');
}
const result = {passed:true, root, validPolicies:3, invalidPolicyTypes:6,
  noScriptExecution:true, ownedConfigurationUnchanged:true, crossBindingRuntimeVerifiedSeparately:true};
writeFileSync(join(root, 'result.json'), JSON.stringify(result, null, 2), {flag:'wx'});
console.log(JSON.stringify(result));
