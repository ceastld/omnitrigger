// 正式模块和共享 SDK 的端口回归；只提供预先声明的内存回执，不实现目录枚举。
import assert from 'node:assert/strict';
import test from 'node:test';
import {runInNewContext} from 'node:vm';
import {existsSync,readFileSync,mkdirSync,mkdtempSync,writeFileSync,lstatSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';

// CI 从仓库根运行。只解析正式树，不接受候选根或环境覆盖。
const repo=process.cwd();
const moduleFile=path.join(repo,'examples/actions/source-file-preview/main.mjs');
const manifestFile=path.join(repo,'examples/actions/source-file-preview/action.json');
const runtime=path.join(repo,'crates/omnitrigger-actions/runtime');
const sharedFiles=['desktop-api.js','browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js','state-api.js'];
const sdkFile=path.join(repo,'tools/js-engine-probe/src/sdk.js');
const hostFile=path.join(runtime,'host.mjs');
const fixtureParent=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../.tmp');
function plain(paths) {
  const checked=new Set();
  for(const file of paths)for(let part=path.resolve(file);;part=path.dirname(part)) {
    assert(!/^(OneDrive.*|Dropbox|iCloud.*)$/i.test(path.basename(part)),'云盘路径不得作为测试输入');
    const info=lstatSync(part);assert(!info.isSymbolicLink(),'测试输入不得经过链接');checked.add(part);
    if(path.dirname(part)===part)break;
  }
  if(process.platform==='win32') {
    const result=spawnSync('powershell.exe',['-NoLogo','-NoProfile','-NonInteractive','-Command',
      "$ErrorActionPreference='Stop';foreach($p in (ConvertFrom-Json $env:SOURCE_PREVIEW_TEST_PATHS)){$i=Get-Item -LiteralPath $p -Force;if(([long]$i.Attributes -band 0x441400) -ne 0){throw 'unsafe test input'}}"],
      {encoding:'utf8',windowsHide:true,timeout:10000,env:{...process.env,SOURCE_PREVIEW_TEST_PATHS:JSON.stringify([...checked])}});
    assert.ifError(result.error);assert.equal(result.status,0,result.stderr);
  }
}
plain([moduleFile,manifestFile,hostFile,sdkFile,...sharedFiles.map(name=>path.join(runtime,name)),existsSync(fixtureParent)?fixtureParent:path.dirname(fixtureParent)]);
const text=file=>readFileSync(file,'utf8');
const shared=name=>text(path.join(runtime,name));
const {default:main}=await import(pathToFileURL(moduleFile));
// 工厂源与宿主共用；原文件是拼接脚本，只补 ESM 导出以直接导入同一个函数。
const factorySource=shared('desktop-api.js')+'\nexport {createDesktopApi};';
const {createDesktopApi:createApi}=await import('data:text/javascript;base64,'+Buffer.from(factorySource).toString('base64'));
const asyncLine=text(hostFile).match(/const asyncApi = [^\n]+;/)?.[0];assert(asyncLine,'正式 Node 宿主缺少 asyncApi');
const asyncApi=runInNewContext(asyncLine+';asyncApi');
mkdirSync(fixtureParent,{recursive:true});
const fixture=mkdtempSync(path.join(fixtureParent,'source-file-preview-'));
const directory=path.join(fixture,'folder'),child=path.join(directory,'child');
mkdirSync(child,{recursive:true});
for(const [parent,name] of [[directory,'a.txt'],[child,'leaf.txt'],[directory,'link']])writeFileSync(path.join(parent,name),'synthetic');
const root=path.win32.normalize(directory),sub=path.win32.normalize(child),file=path.win32.join(root,'a.txt'),leaf=path.win32.join(sub,'leaf.txt');
const row=(name,kind='file',reason=null,parent=root)=>({name,path:path.win32.join(parent,name),kind,reason});
const value=entries=>({entries,complete:true});
const originalTarget=Object.freeze({id:'owned-synthetic-original-target'});
function ports({roots=[root],input,choices=['root:0','open-current'],listing,statOverride,readError,onRead,listingsByPath,selectedPaths=roots}={}) {
  const calls=[],opened=[],menus=[],controller=new AbortController();
  const listings=new Map([[root,value([row('a.txt'),row('child','directory')])],[sub,value([row('leaf.txt','file',null,sub)])]]);
  const kinds=new Map([[root,'directory'],[sub,'directory'],[file,'file'],[leaf,'file']]);
  const queue=[...choices];
  const invoke=async(method,params)=>{
    calls.push({method,params:JSON.parse(JSON.stringify(params)),target:originalTarget});
    if(method==='explorer.context') {assert.deepEqual(JSON.parse(JSON.stringify(params)),{});return {currentDirectory:root,selectedPaths};}
    if(method==='fs.stat')return {kind:statOverride?.(params.path,calls) ?? kinds.get(params.path) ?? 'missing'};
    if(method==='fs.read_directory') {
      assert.equal(params.max_entries,1024);assert([root,sub].includes(params.path));
      onRead?.(controller);
      if(readError)throw Object.assign(Error('PRIVATE native provider text'),{code:readError,method,effectState:'unknown'});
      return listingsByPath?.get(params.path) ?? listing ?? listings.get(params.path);
    }
    if(method==='ui.choose') {
      menus.push(params);assert(queue.length,'发生未预期的菜单请求');
      const answer=queue.shift();return typeof answer==='string'?{status:'selected',id:answer}:answer;
    }
    if(method==='system.open') {opened.push(params.target);return {submitted:true};}
    throw Error('不允许的端口：'+method);
  };
  const api=createApi(invoke);
  const ctx={input:input??{roots},signal:controller.signal,fs:asyncApi(api.fs),ui:asyncApi(api.ui),desktop:asyncApi(api.desktop),
    open:target=>invoke('system.open',{target})};
  for(const key of ['process','packageRoot'])Object.defineProperty(ctx,key,{get(){throw Error('仍依赖 '+key);}});
  return {ctx,calls,menus,opened,controller,queue};
}
const fault=code=>error=>error.code===code&&error.effectState==='not_started'&&!error.message.includes('PRIVATE');
test('实际 Node 模块通过真实共享工厂枚举并打开当前目录，完全不读取 PS/packageRoot',async()=>{
  const p=ports();const output=await main(p.ctx);
  assert.deepEqual(output,{status:'open_submitted',kind:'directory'});
  assert.deepEqual(p.opened,[root]);
  assert.deepEqual(p.calls.filter(c=>c.method==='fs.read_directory').map(c=>c.params),[{path:root,max_entries:1024}]);
  assert.equal(p.menus[1].title,'文件夹预览');assert.equal(p.menus[1].items.length,3);
  assert(p.calls.every(c=>c.target===originalTarget));assert.equal(p.queue.length,0);
});
test('缺 roots 只读取一次原 Explorer；显式 roots 不调用 Explorer',async()=>{
  const p=ports({input:{}});await main(p.ctx);
  assert.equal(p.calls.filter(c=>c.method==='explorer.context').length,1);
  assert.equal(p.calls[0].method,'explorer.context');
  const explicit=ports();await main(explicit.ctx);assert(!explicit.calls.some(c=>c.method==='explorer.context'));
});
test('原 Explorer 空选择及读取错误不回退 Clipboard、前台或 PowerShell',async()=>{
  const empty=ports({input:{},selectedPaths:[]});await assert.rejects(main(empty.ctx),fault('selection_empty'));
  assert.deepEqual(empty.calls.map(c=>c.method),['explorer.context']);
  const p=ports({input:{}});p.ctx.desktop={explorerContext:async()=>{throw Object.assign(Error('synthetic'),{code:'target_changed',effectState:'not_started'});}};
  await assert.rejects(main(p.ctx),fault('target_changed'));assert.equal(p.calls.length,0);
});
test('文件选择保留两次 stat 后直接打开，不需要 readDirectory',async()=>{
  const p=ports({roots:[file],choices:['root:0']});p.ctx.fs={stat:p.ctx.fs.stat};
  assert.deepEqual(await main(p.ctx),{status:'open_submitted',kind:'file'});
  assert.deepEqual(p.calls.map(c=>c.method),['fs.stat','ui.choose','fs.stat','system.open']);
  assert.deepEqual(p.opened,[file]);
});
test('目录优先、进入目录、返回上级均保持根内导航及打开前 stat',async()=>{
  const p=ports({choices:['root:0','entry:0','parent','open-current']});await main(p.ctx);
  assert.deepEqual(p.calls.filter(c=>c.method==='fs.read_directory').map(c=>c.params.path),[root,sub,root]);
  assert.match(p.menus[1].items[1].label,/\[文件夹\] child/);
  assert(p.menus[2].items.some(item=>item.id==='parent'));assert(!p.menus[1].items.some(item=>item.id==='parent'));
  assert.deepEqual(p.calls.slice(-2).map(c=>[c.method,c.params.path??c.params.target]),[['fs.stat',root],['system.open',root]]);
});
test('子文件打开前复验；类型变化时不打开',async()=>{
  const p=ports({choices:['root:0','entry:1']});await main(p.ctx);assert.deepEqual(p.opened,[file]);
  assert.deepEqual(p.calls.slice(-2).map(c=>c.method),['fs.stat','system.open']);
  const changed=ports({choices:['root:0','entry:1'],statOverride:target=>target===file?'missing':undefined});
  await assert.rejects(main(changed.ctx),fault('path_changed'));assert.equal(changed.opened.length,0);
});
test('不可用与 other 子项仍显示，选择后拒绝，不读取或 stat 子项',async()=>{
  for(const [kind,reason] of [['unavailable','reparse_point'],['unavailable','cloud_placeholder'],['unavailable','cloud_path'],['unavailable','unsupported_name'],['other',null]]) {
    const p=ports({listing:value([row('link',kind,reason)]),choices:['root:0','entry:0']});
    await assert.rejects(main(p.ctx),fault('path_unavailable'));
    assert.match(p.menus[1].items[1].label,/\[不可用\]/);
    assert.equal(p.opened.length,0);assert(!p.calls.some(c=>c.method==='fs.stat'&&c.params.path.endsWith('\\link')));
  }
});
test('空目录成功；24 项完整显示而非截断',async()=>{
  for(const count of [0,24]) {
    const p=ports({listing:value(Array.from({length:count},(_,i)=>row('synthetic-'+i)))});
    await main(p.ctx);assert.equal(p.menus[1].items.length,count+1);
    assert(!p.menus[1].title.includes('部分'));
  }
});
test('数量或 64 KiB 平台超限均明确抛 directory_too_large，不显示部分菜单',async()=>{
  for(const scenario of ['entry_count','metadata_bytes']) {
    const p=ports({readError:'directory_too_large'});await assert.rejects(main(p.ctx),fault('directory_too_large'));
    assert.equal(p.menus.length,1,scenario);assert.equal(p.opened.length,0);
    assert.equal(p.calls.filter(c=>c.method==='fs.read_directory').length,1);
  }
});
test('共享工厂拒绝伪完整、超条数及 64 KiB 成功回执，模块不转成 limited',async()=>{
  for(const listing of [{entries:[],complete:false},value(Array.from({length:1025},(_,i)=>row('fake-'+i))),value([row('中'.repeat(12000))])]) {
    const p=ports({listing});await assert.rejects(main(p.ctx),fault('fs_error'));
    assert.equal(p.menus.length,1);assert.equal(p.opened.length,0);
  }
});
test('读取错误类别保留，不回退、不重试、不弹第二个菜单',async()=>{
  for(const code of ['path_missing','not_directory','access_denied','path_unavailable','fs_unavailable','cancelled','timeout']) {
    const p=ports({readError:code});await assert.rejects(main(p.ctx),fault(code));
    assert.equal(p.menus.length,1);assert.equal(p.opened.length,0);
    assert.equal(p.calls.filter(c=>c.method==='fs.read_directory').length,1);
  }
});
test('缺目录 API 明确不可用；缺 process 不影响运行',async()=>{
  const p=ports();p.ctx.fs={stat:p.ctx.fs.stat};await assert.rejects(main(p.ctx),fault('directory_enumeration_unavailable'));
  assert.equal(p.calls.filter(c=>c.method==='fs.read_directory').length,0);assert.equal(p.opened.length,0);
});
test('根菜单或预览取消均不打开；枚举后取消不交付菜单',async()=>{
  const cancelled={status:'cancelled',reason:'dismissed'};
  for(const choices of [[cancelled],['root:0',cancelled]]) {
    const p=ports({choices});assert.deepEqual(await main(p.ctx),{status:'cancelled'});assert.equal(p.opened.length,0);
  }
  const p=ports({onRead:controller=>controller.abort()});await assert.rejects(main(p.ctx),fault('cancelled'));assert.equal(p.menus.length,1);
});
test('未知选择与五轮导航限制不产生意外打开',async()=>{
  const bad=ports({choices:['root:0','entry:999']});await assert.rejects(main(bad.ctx),fault('invalid_response'));assert.equal(bad.opened.length,0);
  const p=ports({choices:['root:0','entry:0','parent','entry:0','parent','entry:0']});
  assert.deepEqual(await main(p.ctx),{status:'navigation_limit',openSubmitted:false});assert.equal(p.opened.length,0);
  assert.equal(p.calls.filter(c=>c.method==='fs.read_directory').length,5);
});
test('消费者保留 1024 路径和大小写去重限制，越界回执失败',async()=>{
  const duplicate=ports({roots:[root,root.toUpperCase()]});await assert.rejects(main(duplicate.ctx),fault('invalid_input'));assert.equal(duplicate.calls.length,0);
  const collision=ports({listing:value([row('a'),row('A')])});await assert.rejects(main(collision.ctx),fault('invalid_response'));assert.equal(collision.menus.length,1);
});
test('入口/清单只有 Node 运行时，无枚举脚本、进程或 Clipboard 回退',()=>{
  const source=text(moduleFile);
  assert(!/process|packageRoot|PowerShell|powershell|list-directory|parse\(result\.stdout\)|listing\.limited/.test(source));
  assert(!/clipboard|captureForeground|readSelectionText/.test(source));
  const manifest=JSON.parse(text(manifestFile));
  assert.deepEqual(manifest.actions.map(action=>[action.runtime,action.entry]),[['node','main.mjs']]);
});

// 分页数量只需合成元数据；不为 120 个端口条目生成真实文件。
function files(count) {
  return Array.from({length:count},(_,index)=>{
    const name=`page-file-${String(index).padStart(3,'0')}.txt`;
    return row(name);
  });
}
test('25 项普通文件夹可翻到第二页并打开最后一个文件；无重新枚举',async()=>{
  const entries=files(25),known=new Set(entries.map(entry=>entry.path));
  const p=ports({listing:value(entries),choices:['root:0','next-page','entry:24'],statOverride:target=>known.has(target)?'file':undefined});
  assert.deepEqual(await main(p.ctx),{status:'open_submitted',kind:'file'});
  assert.deepEqual(p.opened,[entries[24].path]);
  assert.match(p.menus[1].title,/第1\/2页，共25项/);assert.match(p.menus[2].title,/第2\/2页，共25项/);
  assert.equal(p.menus[1].items.filter(item=>item.id.startsWith('entry:')).length,24);
  assert.equal(p.menus[2].items.filter(item=>item.id.startsWith('entry:')).length,1);
  assert(p.menus[1].items.some(item=>item.id==='next-page'));assert(!p.menus[1].items.some(item=>item.id==='previous-page'));
  assert(p.menus[2].items.some(item=>item.id==='previous-page'));assert(!p.menus[2].items.some(item=>item.id==='next-page'));
  assert.equal(p.calls.filter(c=>c.method==='fs.read_directory').length,1);
  assert.deepEqual(p.calls.slice(-2).map(c=>[c.method,c.params.path??c.params.target]),[['fs.stat',entries[24].path],['system.open',entries[24].path]]);
});
test('49 项明确三页，上一页回到同一快照；可显示全部 ID 而不静默丢项',async()=>{
  const entries=files(49),p=ports({listing:value(entries),choices:['root:0','next-page','next-page','previous-page','open-current']});
  await main(p.ctx);
  assert.deepEqual(p.menus.slice(1).map(menu=>menu.items.filter(item=>item.id.startsWith('entry:')).length),[24,24,1,24]);
  const ids=new Set(p.menus.slice(1).flatMap(menu=>menu.items.filter(item=>item.id.startsWith('entry:')).map(item=>item.id)));
  assert.equal(ids.size,49);assert.equal(p.calls.filter(c=>c.method==='fs.read_directory').length,1);
  assert.match(p.menus[4].title,/第2\/3页/);assert(p.calls.every(c=>c.target===originalTarget));
});
test('分页后的目录导航及返回上级重新 stat/读快照，并复位页号',async()=>{
  const dirs=Array.from({length:24},(_,index)=>row(`a${String(index).padStart(3,'0')}`,'directory'));
  dirs.push(row('child','directory'));
  const listingsByPath=new Map([[root,value(dirs)],[sub,value([row('leaf.txt','file',null,sub)])]]);
  const p=ports({listingsByPath,choices:['root:0','next-page','entry:24','parent','open-current']});
  await main(p.ctx);
  assert.deepEqual(p.calls.filter(c=>c.method==='fs.read_directory').map(c=>c.params.path),[root,sub,root]);
  assert.match(p.menus[4].title,/第1\/2页/);
  for(const [index,call] of p.calls.entries())if(call.method==='fs.read_directory') {
    assert.equal(p.calls[index-1].method,'fs.stat');assert.equal(p.calls[index-1].params.path,call.params.path);
  }
});
test('不能选择当前页之外的 ID；页边界控制不能绕过验证',async()=>{
  for(const choices of [['root:0','entry:24'],['root:0','previous-page'],['root:0','next-page','next-page']]) {
    const p=ports({listing:value(files(25)),choices});await assert.rejects(main(p.ctx),fault('invalid_response'));assert.equal(p.opened.length,0);
  }
});
test('翻页计入五轮；120 项不能无限翻页或重置计数',async()=>{
  const p=ports({listing:value(files(120)),choices:['root:0','next-page','next-page','next-page','next-page','previous-page']});
  assert.deepEqual(await main(p.ctx),{status:'navigation_limit',openSubmitted:false});
  assert.equal(p.menus.length,6);assert.equal(p.calls.filter(c=>c.method==='fs.read_directory').length,1);
  assert.equal(p.calls.length,9);assert.equal(p.opened.length,0);
});
test('分页中取消不会重读目录；已缓存条目在打开前类型变化仍失败',async()=>{
  const entries=files(25),p=ports({listing:value(entries),choices:['root:0','next-page',{status:'cancelled',reason:'dismissed'}]});
  assert.deepEqual(await main(p.ctx),{status:'cancelled'});assert.equal(p.calls.filter(c=>c.method==='fs.read_directory').length,1);
  const changed=ports({listing:value(entries),choices:['root:0','next-page','entry:24']});
  await assert.rejects(main(changed.ctx),fault('path_changed'));assert.equal(changed.opened.length,0);
});
function realBudgetContext(prefill=0) {
  const calls=[],menus=[],choices=['root:0','next-page','next-page','next-page','next-page','previous-page'];
  const entries=files(120);
  const sharedSource=['desktop-api.js','browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js','state-api.js'].map(shared).join('\n');
  const scope={ctx:{input:{roots:[root]},__nativeContext:{connection:{port:1,token:'synthetic'}}},__hostCall:serialized=>{
    const frame=JSON.parse(serialized);calls.push(frame);
    let reply=null;
    if(frame.kind==='delay')return JSON.stringify({ok:true,value:null});
    if(frame.method==='fs.stat')reply={kind:'directory'};
    else if(frame.method==='fs.read_directory'){assert.equal(frame.params.max_entries,1024);reply=value(entries);}
    else if(frame.method==='ui.choose'){menus.push(frame.params);assert(choices.length);reply={status:'selected',id:choices.shift()};}
    else throw Error('额外宿主端口 '+frame.method);
    return JSON.stringify({ok:true,value:reply});
  }};
  runInNewContext(sharedSource+text(sdkFile),scope);
  for(let index=0;index<prefill;index++)scope.ctx.delay(0);
  // 在 Node 中使用现有 asyncApi 包装真实同步 SDK，仅验证其32预算；不称 Script 支持异步入口。
  const ctx={...scope.ctx,signal:new AbortController().signal,fs:asyncApi(scope.ctx.fs),ui:asyncApi(scope.ctx.ui),desktop:asyncApi(scope.ctx.desktop),open:async(...args)=>scope.ctx.open(...args)};
  return {ctx,calls,menus};
}
test('实际共享 SDK 的32调用预算对翻页生效；已有调用不会被入口重置',async()=>{
  const p=realBudgetContext();assert.deepEqual(await main(p.ctx),{status:'navigation_limit',openSubmitted:false});assert.equal(p.calls.length,9);
  const exhausted=realBudgetContext(24);
  await assert.rejects(main(exhausted.ctx),error=>error.message==='单次脚本最多调用 32 次宿主 API');
  assert.equal(exhausted.calls.length,32);assert.equal(exhausted.menus.length,5);
  assert.equal(exhausted.calls.filter(call=>call.method==='fs.read_directory').length,1);
});
test('API 硬上限仍不转换为分页部分结果，不增加预览/打开副作用',async()=>{
  const p=ports({readError:'directory_too_large'});await assert.rejects(main(p.ctx),fault('directory_too_large'));
  assert.equal(p.menus.length,1);assert.equal(p.opened.length,0);
});
test('实际共享 SDK 下四根、Explorer与五轮最深导航加打开共27调用，未越32额度',async()=>{
  const directories=[root];
  for(let depth=1;depth<5;depth++)directories.push(path.win32.join(directories.at(-1),`step-${depth}`));
  const endFile=path.win32.join(directories.at(-1),'finish.txt');
  const extras=['extra-a','extra-b','extra-c'].map(name=>path.win32.join(fixture,name));
  const listings=new Map(directories.map((dir,index)=>{
    const child=index===4?endFile:directories[index+1];
    return [dir,value([{name:path.win32.basename(child),path:child,kind:index===4?'file':'directory',reason:null}])];
  }));
  const calls=[],choices=['root:0',...Array(5).fill('entry:0')];
  const scope={ctx:{input:{},__nativeContext:{connection:{port:1,token:'synthetic'}}},__hostCall:serialized=>{
    const frame=JSON.parse(serialized);calls.push(frame);let result;
    if(frame.method==='explorer.context')result={currentDirectory:root,selectedPaths:[root,...extras]};
    else if(frame.method==='fs.stat')result={kind:frame.params.path===endFile?'file':'directory'};
    else if(frame.method==='fs.read_directory')result=listings.get(frame.params.path);
    else if(frame.method==='ui.choose'){assert(choices.length);result={status:'selected',id:choices.shift()};}
    else if(frame.kind==='native'&&frame.method==='system.open'){assert.deepEqual(frame.params,{target:endFile});result={submitted:true};}
    else throw Error('额外宿主端口 '+frame.method);
    return JSON.stringify({ok:true,value:result});
  }};
  runInNewContext(['desktop-api.js','browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js','state-api.js'].map(shared).join('\n')+text(sdkFile),scope);
  // 这里只用同步SDK核对32次额度；Node流程要求submitted回执，Script成功契约为null。
  // 先由真实Script SDK验证桥接回执，再显式适配返回值，不绕过SDK或少计打开调用。
  const ctx={...scope.ctx,signal:new AbortController().signal,fs:asyncApi(scope.ctx.fs),ui:asyncApi(scope.ctx.ui),desktop:asyncApi(scope.ctx.desktop),open:async(...args)=>{
    assert.equal(scope.ctx.open(...args),null);return {submitted:true};
  }};
  assert.deepEqual(await main(ctx),{status:'open_submitted',kind:'file'});assert.equal(calls.length,27);
  assert.equal(calls.filter(call=>call.method==='fs.read_directory').length,5);assert.equal(choices.length,0);
  assert.equal(calls.filter(call=>call.method==='system.open').length,1);
  assert(calls.every(call=>call.kind==='native'));
});
