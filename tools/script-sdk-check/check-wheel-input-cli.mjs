// 负向参数检查只搭配无界面入口，验证拒绝发生在配置读取或其他操作之前。
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdirSync,mkdtempSync,readFileSync,readdirSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const app=resolve(process.argv[2]??'target/debug/omnitrigger.exe');
mkdirSync(join(repo,'.tmp'),{recursive:true});
const root=mkdtempSync(join(repo,'.tmp/wheel-cli-check-'));
const config=join(root,'must-not-load.json');
writeFileSync(config,'这是不能作为配置解析的验收哨兵。');
const flag='--wheel-input-policy';
const cases=[
  ['--check-config',flag],
  [flag,'unknown','--check-config'],
  [flag,'unfiltered',flag,'small-vertical-150ms','--check-config'],
  [flag,'unfiltered','--check-config'],
  [flag,'small-vertical-150ms','--list-package-actions'],
  [flag,'small-vertical-150ms','--create-script-action','must-not-create'],
  [flag,'small-vertical-150ms','--run-action',join(root,'missing-call.json')],
  [flag,'small-vertical-150ms','--notification-smoke',join(root,'missing-notification-directory')],
];
for(const args of cases){
  const child=spawnSync(app,['--config',config,...args],{encoding:'utf8',windowsHide:true,timeout:15000,maxBuffer:64*1024});
  assert.ifError(child.error);
  assert.equal(child.status,1,child.stderr||child.stdout);
  let response;
  try { response=JSON.parse(child.stderr.trim()); }
  catch { assert.fail('策略参数须在读取哨兵配置或执行操作前结构化拒绝：'+child.stderr); }
  assert.equal(response.status,'rejected');
  assert.equal(response.error?.code,'invalid_cli_arguments');
  assert.equal(response.error?.effect_state,'not_started');
  assert.equal(readFileSync(config,'utf8'),'这是不能作为配置解析的验收哨兵。');
  assert.deepEqual(readdirSync(root),['must-not-load.json']);
}
console.log(JSON.stringify({passed:true,cases:cases.length,noGuiModeRequested:true,ownedConfigurationUnchanged:true,root}));
