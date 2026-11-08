// 普通应用的显式状态CLI验收；只使用全新配置和Node counter，不调用桌面或剪贴板API。
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync} from 'node:fs';
import {basename, dirname, isAbsolute, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';

assert.equal(process.argv.length,4,'用法：node action-state-cli.test.mjs --app <正常构建应用的绝对路径>');
assert.equal(process.argv[2],'--app','必须显式指定 --app，不选择默认应用或Daily');
assert(isAbsolute(process.argv[3]),'--app 必须是绝对路径');
const application=resolve(process.argv[3]);
assert(!basename(application).toLowerCase().includes('daily'),'不能对Daily执行验收');
function local(path) {
  for (let part=resolve(path);;) {
    assert(!/^(onedrive|icloud)/i.test(basename(part)) && !/^dropbox$/i.test(basename(part)),'验收不使用云盘目录');
    if (existsSync(part)) assert(!lstatSync(part).isSymbolicLink(),'验收不跟随链接');
    const parent=dirname(part);if(parent===part)break;part=parent;
  }
}
local(application);assert(lstatSync(application).isFile(),'--app 不是普通文件');
const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');

test('显式CLI状态：独立进程durable计数1→2，默认无存储，非法组合提前拒绝', {timeout:360000}, () => {
  const parent=join(repo,'.tmp');local(parent);mkdirSync(parent,{recursive:true});
  const root=mkdtempSync(join(parent,'action-state-cli-'));
  const config=join(root,'bindings.json'),state=join(root,'action-state');
  const folder=join(root,'actions','counter');mkdirSync(folder,{recursive:true});
  writeFileSync(config,'[]',{flag:'wx'});
  const entry=`export default async function counter(ctx) {
    let before;
    try { before = await ctx.state.read('durable','counter'); }
    catch (error) { return {stateError:{code:error.code,method:error.method,effectState:error.effectState}}; }
    const count = before.entry.present ? before.entry.value : 0;
    if (!Number.isSafeInteger(count) || count < 0) throw Error('无效计数');
    const result = await ctx.state.compareAndSwap('durable','counter',before.revision,{present:true,value:count+1});
    if (result.status !== 'committed') throw Error('CAS冲突，不重试');
    return {status:result.status,count:result.current.entry.value};
  }\n`;
  const manifest={schema_version:1,id:'cli-state-counter',name:'隔离状态CLI验收',version:'1.0.0',
    actions:[{id:'increment',title:'增加一次',version:1,runtime:'node',entry:'main.mjs'}]};
  const call={package_id:manifest.id,action_id:'increment',action_version:1,input:{}};
  const entryPath=join(folder,'main.mjs'),manifestPath=join(folder,'action.json'),callPath=join(root,'call.json');
  writeFileSync(entryPath,entry,{flag:'wx'});
  writeFileSync(manifestPath,JSON.stringify(manifest),{flag:'wx'});
  writeFileSync(callPath,JSON.stringify(call),{flag:'wx'});
  const env={...process.env,OMNITRIGGER_NODE:process.execPath};
  delete env.OMNITRIGGER_SCRIPT_HOST;delete env.NODE_OPTIONS;delete env.NODE_PATH;
  const invocations=[];
  function cli(args,code=0) {
    const child=spawnSync(application,['--config',config,...args],{
      windowsHide:true,encoding:'utf8',timeout:code===0?60000:10000,maxBuffer:1024*1024,env,
    });
    assert.ifError(child.error);assert.equal(child.signal,null);assert.equal(child.status,code,child.stderr||child.stdout);
    assert.equal(readFileSync(config,'utf8'),'[]');assert.equal(readFileSync(entryPath,'utf8'),entry);
    assert.deepEqual(JSON.parse(readFileSync(manifestPath,'utf8')),manifest);
    assert.deepEqual(JSON.parse(readFileSync(callPath,'utf8')),call);
    const output=JSON.parse(code===0?child.stdout:child.stderr);
    assert.equal(code===0?child.stderr.trim():child.stdout.trim(),'');
    invocations.push({args,pid:child.pid,exitCode:child.status});return output;
  }

  // 每个组合都必须在读调用文件、创建工程或安装运行时之前拒绝。
  const invalid=[
    ['--with-action-state'],
    ['--with-action-state=true'],
    ['--run-action',callPath,'--with-action-state','false'],
    ['--run-action',callPath,'--with-action-state','--with-action-state'],
    ['--check-action',callPath,'--with-action-state'],
    ['--check-config','--with-action-state'],
    ['--list-package-actions','--with-action-state'],
    ['--create-script-action','must-not-create','--with-action-state'],
    ['--install-script-runtime',join(root,'missing.zip'),'--sha256','0'.repeat(64),'--with-action-state'],
    // 既有preview解析器也会先拒绝未知flag，不会显示浮层；这里要求统一入口返回结构化错误。
    ['--preview-system-hud','brightness','--with-action-state'],
  ];
  for (const args of invalid) {
    const result=cli(args,1);
    assert.equal(result.status,'rejected');assert.equal(result.error.code,'invalid_cli_arguments');
    assert.equal(result.error.effect_state,'not_started');assert.equal(existsSync(state),false);
    assert.deepEqual(readdirSync(join(root,'actions')),['counter']);
    assert.equal(existsSync(join(root,'runtimes')),false);
  }

  const unavailable=cli(['--run-action',callPath]);
  assert.equal(unavailable.status,'succeeded');
  assert.deepEqual(unavailable.output,{stateError:{code:'state_unavailable',method:'state.read',effectState:'not_started'}});
  assert.equal(existsSync(state),false,'默认CLI不能创建状态目录');

  const first=cli(['--run-action',callPath,'--with-action-state']);
  assert.equal(first.status,'succeeded');assert.deepEqual(first.output,{status:'committed',count:1});
  assert.equal(existsSync(state),true);
  const documents=readdirSync(state).filter(name=>/^[0-9a-f]{64}\.json$/.test(name));
  assert.equal(documents.length,1,'只有本轮真实动作作用域的durable文档');
  const stored=join(state,documents[0]),firstBytes=readFileSync(stored);
  const firstPid=invocations.at(-1).pid;
  const second=cli(['--with-action-state','--run-action',callPath]);
  assert.equal(second.status,'succeeded');assert.deepEqual(second.output,{status:'committed',count:2});
  // 两次spawnSync均已完成退出；Windows可以复用PID，不能用PID不等证明进程重建。
  const secondPid=invocations.at(-1).pid;assert(firstPid>0 && secondPid>0);
  const secondBytes=readFileSync(stored);assert.notDeepEqual(secondBytes,firstBytes);

  const again=cli(['--run-action',callPath]);
  assert.deepEqual(again.output,unavailable.output);
  assert.deepEqual(readFileSync(stored),secondBytes,'默认CLI不能隐式挂载或修改已有durable文档');
  writeFileSync(join(root,'result.json'),JSON.stringify({passed:true,application,root,
    durableCounts:[first.output.count,second.output.count],invocationPids:[firstPid,secondPid],separateCompletedCliProcesses:true,
    defaultStateUnavailable:true,invalidCombinations:invalid.length,invocations},null,2)+'\n',{flag:'wx'});
});
