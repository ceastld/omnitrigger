// 经真实 Node 动作服务和系统打开适配运行自有无窗口夹具，不打开浏览器或个人文件。
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {constants, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {setTimeout} from 'node:timers/promises';

assert.equal(process.platform, 'win32', '此检查使用 Windows 系统打开适配。');
assert(process.argv[2] && process.argv[3], '请提供应用和已编译的 file-open-probe.rs 夹具。');
const app = resolve(process.argv[2]), fixture = resolve(process.argv[3]);
const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
mkdirSync(join(repo, '.tmp'), {recursive:true});
const root = mkdtempSync(join(repo, '.tmp/node-open-check-'));
const packageRoot = join(root, 'actions/open-check');
const fixtureRoot = join(root, '自有测试程序 space');
mkdirSync(packageRoot, {recursive:true});
mkdirSync(fixtureRoot);
const target = join(fixtureRoot, 'owned-open-probe.exe');
copyFileSync(fixture, target, constants.COPYFILE_EXCL);
const marker = join(fixtureRoot, '.opened.json');
assert(!existsSync(marker));
const config = join(root, 'bindings.json');
writeFileSync(config, '[]');
writeFileSync(join(packageRoot, 'action.json'), JSON.stringify({
  schema_version:1, id:'open-check', name:'系统打开验收', version:'1.0.0',
  actions:[{id:'check',title:'打开自有无窗口夹具',version:1,runtime:'node',entry:'main.mjs'}],
}));
const invalid = ['javascript:owned-invalid', 'file://owned-invalid', 'relative-owned.txt', '', null, ' about:blank', 'about:blank\0'];
writeFileSync(join(packageRoot, 'main.mjs'), `export default async function main(ctx) {
  const rejected = [];
  for (const target of ctx.input.invalid) {
    try { await ctx.open(target); throw Error('无效目标意外通过'); }
    catch (error) {
      if (error.code !== 'invalid_input' || error.method !== 'system.open'
          || error.effectState !== 'not_started') throw error;
      rejected.push({code:error.code,method:error.method,effectState:error.effectState});
    }
  }
  return {rejected,opened:await ctx.open(ctx.input.target)};
}`);
const call = join(root, 'call.json');
writeFileSync(call, JSON.stringify({package_id:'open-check',action_id:'check',action_version:1,input:{invalid,target}}));
const child = spawnSync(app, ['--config',config,'--run-action',call], {
  encoding:'utf8',windowsHide:true,timeout:60000,maxBuffer:1024*1024,
  env:{...process.env,OMNITRIGGER_NODE:process.execPath},
});
assert.ifError(child.error);
assert.equal(child.status,0,child.stderr || child.stdout);
const terminal = JSON.parse(child.stdout);
assert.equal(terminal.status,'succeeded',JSON.stringify(terminal));
assert.deepEqual(terminal.output,{rejected:invalid.map(()=>({code:'invalid_input',method:'system.open',effectState:'not_started'})),opened:{submitted:true}});
const deadline = Date.now()+5000;
let opened;
while (Date.now()<deadline) {
  if (existsSync(marker)) {
    try { opened = JSON.parse(readFileSync(marker,'utf8')); break; }
    catch (error) { if (!(error instanceof SyntaxError)) throw error; }
  }
  await setTimeout(20);
}
assert(opened,'系统接纳后，自有无窗口程序没有写入本轮标记。');
assert.equal(opened.opened,true);
assert(Number.isSafeInteger(opened.process_id) && opened.process_id>0);
assert.equal(readFileSync(config,'utf8'),'[]');
const result = {passed:true,root,invalidRejected:invalid.length,systemAccepted:true,ownedProbeStarted:true,
  ownedProbePid:opened.process_id,noUi:true,personalConfigUsed:false};
writeFileSync(join(root,'result.json'),JSON.stringify(result,null,2));
console.log(JSON.stringify(result));
