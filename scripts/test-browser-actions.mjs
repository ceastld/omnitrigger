// 真实 Chrome 验收：复用已注册的 Native Host，仅操作本次创建的隔离 profile。
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { cp, mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { resolve, join, relative } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { setTimeout as delay } from 'node:timers/promises';
import { verifyDesktopShortcuts } from './browser-shortcut-smoke.mjs';
import { verifyBrowserWorkflows } from './browser-workflows-smoke.mjs';

const args = process.argv.slice(2);
const option = name => { const index = args.indexOf(name); if (index < 0 || !args[index + 1]) throw Error(`需要 ${name}`); return resolve(args[index + 1]); };
const connector = option('--connector');
const extension = option('--extension');
const playwright = createRequire(import.meta.url)(option('--playwright-module'));
const chrome = option('--chrome');
const root = resolve(fileURLToPath(new URL('..', import.meta.url)));
await mkdir(join(root, 'output/playwright'), { recursive: true });
const scratch = await mkdtemp(join(root, 'output/playwright/browser-actions-'));
const report = { ok: false, checks: [], scope: '隔离 Chrome、本地页面、已安装 SDK' };
let extensionPath = extension;
if (args.includes('--workflow-recipes')) {
  option('--app');
  // 只在本轮临时副本声明测试所需权限，不操作已安装扩展及用户浏览器设置。
  extensionPath = join(scratch, 'workflow-extension');
  await cp(extension, extensionPath, { recursive: true, errorOnExist: true, force: false });
  const manifest = JSON.parse(await readFile(join(extensionPath, 'manifest.json'), 'utf8'));
  const extra = ['tabs', 'sessions'].filter(name => !manifest.permissions.includes(name));
  report.permissionFixture = { original: manifest.permissions, added: extra };
  manifest.permissions = [...manifest.permissions, ...extra];
  manifest.optional_permissions = (manifest.optional_permissions ?? []).filter(name => !extra.includes(name));
  await writeFile(join(extensionPath, 'manifest.json'), JSON.stringify(manifest, null, 2));
}
const { connect } = await import(pathToFileURL(join(connector, 'sdk-js/index.mjs')));
const config = JSON.parse(await readFile(join(connector, 'connection.json'), 'utf8'));
const client = await connect({ pipe: config.pipe, timeoutMs: 3000 });
let context;
let profile;
const server = createServer((request, response) => { response.setHeader('Content-Type', 'text/html; charset=utf-8'); response.end(`<title>OmniTrigger ${request.url}</title><h1>浏览器动作测试</h1><button type="button">测试交互</button>`); });
try {
  const existing = await client.profiles();
  report.existingProfiles = existing.length;
  const browserEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
    !['OMNITRIGGER_BROWSER_CONFIG','OMNITRIGGER_BROWSER_PIPE'].includes(key.toUpperCase())));
  context = await playwright.chromium.launchPersistentContext(join(scratch, 'profile'), {
    executablePath: chrome, headless: !args.includes('--desktop-shortcuts'), env: browserEnv, ignoreDefaultArgs: ['--disable-extensions'],
    args: ['--enable-unsafe-extension-debugging', '--no-first-run', '--no-default-browser-check'],
  });
  const cdp = await context.browser().newBrowserCDPSession();
  const loaded = await cdp.send('Extensions.loadUnpacked', { path: extensionPath });
  report.chromeVersion = context.browser().version();
  report.extensionId = loaded.id;
  let worker = context.serviceWorkers().find(item => item.url().startsWith(`chrome-extension://${loaded.id}/`));
  worker ??= await context.waitForEvent('serviceworker', {
    predicate: item => item.url().startsWith(`chrome-extension://${loaded.id}/`), timeout: 15000,
  });
  assert(worker.url().startsWith(`chrome-extension://${loaded.id}/`), '只能读取本次加载的扩展 worker');
  // 新隔离 profile 默认启用，由 background 初始化连接；普通标签中的 popup 消息会被 sender.tab 拒绝。
  report.connectionBootstrap = 'fresh-profile-default-enabled; own-worker-only; no popup messages';
  report.connectionHistory = [];
  let connected = false;
  for (let attempt = 0; attempt < 300; attempt++) {
    const identity = await worker.evaluate(() => chrome.storage.local.get({browserInstanceId:'',bridgeEnabled:true}));
    const identityId = identity.browserInstanceId;
    assert(!existing.some(item => item.browser_instance_id === identityId), '隔离 worker 身份在启动前已经存在，停止路由');
    profile = typeof identityId === 'string' && identityId ? identityId : undefined;
    const status = await worker.evaluate(() => chrome.storage.session.get('bridgeStatus'));
    const matching = profile ? (await client.profiles()).filter(item => item.browser_instance_id === profile) : [];
    const sample = { ...status, profile, bridgeEnabled:identity.bridgeEnabled,
      matchingProfiles:matching.map(item => ({browser_instance_id:item.browser_instance_id,session_id:item.session_id})) };
    report.connectionStatus = sample;
    const previous = report.connectionHistory.at(-1);
    if (!previous || JSON.stringify(previous) !== JSON.stringify(sample)) {
      report.connectionHistory.push(sample);
      if (report.connectionHistory.length > 40) report.connectionHistory.splice(1,1);
    }
    const bridge = status.bridgeStatus;
    if (profile && bridge?.state === 'connected' && bridge.browser_instance_id === profile
        && typeof bridge.session_id === 'string' && bridge.session_id
        && matching.length === 1 && matching[0].session_id === bridge.session_id) {
      connected = true;
      report.connectedSessionId = bridge.session_id;
      break;
    }
    await delay(100);
  }
  assert(profile && !existing.some(item => item.browser_instance_id === profile), '必须唯一定位新建的隔离 profile');
  assert(connected, '自建扩展尚未连接；查看 connectionStatus/connectionHistory，不更改注册或模拟 popup');
  report.checks.push('Native Host 连接及明确 profile 路由');
  const call = (method, args = [], options = {}) => client.call(method, args, { profile, ...options });
  const tree = await call('bookmarks.getTree');
  assert(Array.isArray(tree) && tree[0].id);
  assert(Array.isArray(await call('bookmarks.getChildren', [tree[0].id])));
  assert(Array.isArray(await call('bookmarks.search', [{query: 'OmniTrigger 不存在的测试书签'}])));
  report.checks.push('bookmarks.getTree/getChildren/search');
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const tab = await call('tabs.create', [{ url: base + '/one', active: false }]);
  assert(Number.isInteger(tab.id));
  for (let attempt = 0; attempt < 50; attempt++) {
    if (context.pages().some(page => page.url() === base + '/one')) break;
    await delay(100);
  }
  const initialPage = context.pages().find(page => page.url() === base + '/one');
  assert(initialPage, '首个测试页面尚未打开');
  await initialPage.waitForLoadState();
  await initialPage.getByRole('button', {name:'测试交互'}).click();
  assert.equal((await call('tabs.get', [tab.id])).id, tab.id);
  assert((await call('tabs.query', [{}])).some(item => item.id === tab.id));
  assert.equal((await call('tabs.update', [tab.id, {pinned: true, muted: true}])).pinned, true);
  assert.equal((await call('tabs.get', [tab.id])).mutedInfo.muted, true);
  await call('tabs.update', [tab.id, {pinned: false, muted: false}]);
  await call('tabs.update', [tab.id, {url: base + '/two'}]);
  for (let attempt = 0; attempt < 50; attempt++) {
    if (context.pages().some(page => page.url() === base + '/two')) break;
    await delay(100);
  }
  const page = context.pages().find(page => page.url() === base + '/two');
  assert(page, '标签页导航尚未完成');
  await page.waitForLoadState();
  assert.equal(await page.title(), 'OmniTrigger /two');
  const pageCdp = await context.newCDPSession(page);
  report.historyBeforeBack = await pageCdp.send('Page.getNavigationHistory');
  try { await call('tabs.goBack', [tab.id]); }
  catch (error) {
    report.nativeBackComparison = await worker.evaluate(async id => {
      try { await chrome.tabs.goBack(id); return {ok:true}; }
      catch (error) {return {ok:false,message:error.message};}
    }, tab.id);
    throw error;
  }
  await page.waitForURL(base + '/one');
  await page.waitForLoadState();
  report.historyAfterBack = await pageCdp.send('Page.getNavigationHistory');
  await call('tabs.goForward', [tab.id]);
  await page.waitForURL(base + '/two');
  await call('tabs.reload', [tab.id]);
  const copy = await call('tabs.duplicate', [tab.id]);
  assert(copy.id !== tab.id);
  assert.equal(await call('tabs.remove', [[copy.id, tab.id]]), undefined);
  assert(!(await call('tabs.query', [{}])).some(item => [copy.id, tab.id].includes(item.id)));
  report.checks.push('标签页新建/查询/获取/导航/前后退/刷新/复制/固定/静音/关闭及后置核验');
  const window = await call('windows.create', [{url: base + '/window', focused: false}]);
  assert(Number.isInteger(window.id));
  await call('windows.remove', [window.id]);
  report.checks.push('windows.create/remove（仅测试窗口）');
  if (args.includes('--desktop-shortcuts')) {
    report.desktopShortcuts = await verifyDesktopShortcuts({context,cdp,call,base,scratch,app:option('--app'),root,fixture:option('--desktop-shortcuts')});
    report.checks.push('隔离 Chrome：真实桌面脚本 API 完成上一标签、下一标签及关闭当前标签');
  }
  await assert.rejects(call('omnitriggerMissing.noSuchMethod'), error => { report.apiError = {code:error.code, effectState:error.effectState}; return !!error.code; });
  await assert.rejects(call('tabs.get', [2147483647]), error => error.code === 'chrome_error');
  const cancellation = new AbortController(); cancellation.abort();
  const count = (await call('tabs.query', [{}])).length;
  await assert.rejects(call('tabs.create', [{url:base + '/cancelled'}], {signal:cancellation.signal}), error => error.code === 'cancelled' && error.effectState === 'not_started');
  assert.equal((await call('tabs.query', [{}])).length, count);
  report.checks.push('未知 API、无效标签页、派发前取消且没有新建标签页');
  // 在隔离 profile 中放置合成大结果，验证 SDK 分块及脚本内处理，最终只返回摘要。
  await worker.evaluate(() => chrome.storage.local.set({omnitriggerActionFixture: '中文🦀'.repeat(100000)}));
  const host = (await Promise.all(['desktop-api.js','browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js','host.mjs'].map(name => readFile(join(root,'crates/omnitrigger-actions/runtime',name),'utf8')))).join('\n');
  const entry = join(scratch, 'main.mjs');
  await writeFile(entry, `export default async ctx => {
    const profiles = await ctx.browser.profiles();
    const value = await ctx.browser.call('storage.local.get', ['omnitriggerActionFixture'], {profile:ctx.input.profile});
    return { connected:profiles.some(p=>p.browser_instance_id===ctx.input.profile), characters:value.omnitriggerActionFixture.length };
  }`);
  const responsePath = join(scratch, 'response.json');
  const child = spawn(process.execPath, ['--input-type=module','--eval',host], {windowsHide:true, stdio:['pipe','ignore','pipe'], env:{...process.env, OMNITRIGGER_BROWSER_PIPE:''}});
  const exit = new Promise((resolve, reject) => { child.once('error',reject); child.once('exit',code=>code===0?resolve():reject(Error(`宿主退出 ${code}`))); });
  let stderr = ''; child.stderr.on('data', chunk => { stderr += chunk; });
  child.stdin.write(JSON.stringify({api_version:1, invocation:{invocation_id:'1',binding_id:'0'}, input:{profile}, package_root:scratch, entry_path:entry, response_path:responsePath, timeout_ms:10000,
    browser:{sdk_path:join(connector,'sdk-js/index.mjs'),config_path:join(connector,'connection.json')}})+'\n');
  const watchdog = setTimeout(() => child.kill(), 15000);
  try { await exit; } catch(error) { throw Error(`${error.message}: ${stderr}`); } finally {clearTimeout(watchdog);}
  const result = JSON.parse(await readFile(responsePath,'utf8'));
  assert.equal(result.status,'ok',JSON.stringify(result));
  assert.deepEqual(result.output,{connected:true,characters:400000});
  report.checks.push('真实 ctx.browser 宿主处理约 1 MiB 合成 Unicode 结果，最终返回摘要');
  if (args.includes('--app')) {
    const app = option('--app');
    const packageRoot = join(scratch, 'actions/browser-test');
    const runtime = join(scratch, 'plugins/browser-connector/runtime/0.1.0');
    await mkdir(packageRoot, {recursive:true});
    await mkdir(join(runtime, 'sdk-js'), {recursive:true});
    await cp(join(connector,'sdk-js'),join(runtime,'sdk-js'),{recursive:true,errorOnExist:true,force:false});
    await writeFile(join(runtime,'connection.json'),await readFile(join(connector,'connection.json')));
    await writeFile(join(packageRoot,'main.mjs'),await readFile(entry));
    await writeFile(join(packageRoot,'action.json'),JSON.stringify({schema_version:1,id:'browser-test',name:'浏览器验收',version:'1.0.0',
      actions:[{id:'run',title:'浏览器验收',version:1,runtime:'node',entry:'main.mjs'}]}));
    const callPath = join(scratch,'call.json');
    await writeFile(callPath,JSON.stringify({package_id:'browser-test',action_id:'run',action_version:1,input:{profile}}));
    const cli = spawn(app,['--config',relative(process.cwd(),join(scratch,'bindings.json')),'--run-action',callPath],{windowsHide:true,stdio:['ignore','pipe','pipe']});
    let stdout = ''; let stderr = '';
    cli.stdout.on('data',chunk=>{stdout+=chunk;}); cli.stderr.on('data',chunk=>{stderr+=chunk;});
    const timer = setTimeout(()=>cli.kill(),20000);
    try {await new Promise((resolve,reject)=>{cli.once('error',reject);cli.once('exit',code=>code===0?resolve():reject(Error(`应用调用失败 ${code}: ${stderr}`)));});}
    finally {clearTimeout(timer);}
    const output = JSON.parse(stdout);
    assert.equal(output.status,'succeeded');
    assert.deepEqual(output.output,{connected:true,characters:400000});
    report.checks.push('真实 OmniTrigger --run-action → Rust 配置核验 → Node ctx.browser → Chrome → 摘要结果');

    if (args.includes('--workflow-recipes')) {
      report.workflows = await verifyBrowserWorkflows({ call, base, scratch, app, root, profile });
      report.checks.push('真实浏览器脚本：网址新建/复用、明确标签复制、书签定位、指定会话恢复及歧义拒绝');
    }

    if (args.includes('--script-host')) {
      for (const name of ['omnitrigger-browser.exe','omnitrigger-browser-host.exe']) {
        await writeFile(join(runtime,name),await readFile(join(connector,name)));
      }
      await writeFile(join(scratch,'browser-connector.json'),JSON.stringify({enabled:true,registered:true,installed:true,extension_id:loaded.id,version:'0.1.0',cli_path:join(runtime,'omnitrigger-browser.exe')}));
      const light = join(scratch,'actions/light-browser-test');
      await mkdir(light,{recursive:true});
      await writeFile(join(light,'action.json'),JSON.stringify({schema_version:1,id:'light-browser-test',name:'轻量浏览器验收',version:'1.0.0',actions:[{id:'run',title:'轻量浏览器验收',version:1,runtime:'script',entry:'main.js'}]}));
      await writeFile(join(light,'main.js'),`function main(ctx) {
        const options={profile:ctx.input.profile};
        const profiles=ctx.browser.profiles();
        const tree=ctx.browser.call('bookmarks.getTree',[],options);
        const value=ctx.browser.call('storage.local.get',['omnitriggerActionFixture'],options);
        const tab=ctx.browser.call('tabs.create',[{url:ctx.input.url,active:false}],options);
        const absent=ctx.browser.call('tabs.remove',[tab.id],options)===undefined;
        let apiError;
        try {ctx.browser.call('omnitriggerMissing.noSuchMethod',[],options)}
        catch(e) {apiError={code:e.code,effectState:e.effectState,method:e.method};}
        return {connected:profiles.some(p=>p.browser_instance_id===ctx.input.profile),characters:value.omnitriggerActionFixture.length,rootId:tree[0].id,undefinedPreserved:absent,apiError};
      }`);
      const lightCall=join(light,'call.json');
      await writeFile(lightCall,JSON.stringify({package_id:'light-browser-test',action_id:'run',action_version:1,input:{profile,url:base+'/light-script'}}));
      const command=spawn(app,['--config',relative(process.cwd(),join(scratch,'bindings.json')),'--run-action',lightCall],{windowsHide:true,stdio:['ignore','pipe','pipe'],env:{...process.env,OMNITRIGGER_SCRIPT_HOST:option('--script-host'),OMNITRIGGER_NODE:join(scratch,'node-not-installed.exe')}});
      let output='';let errors='';command.stdout.on('data',chunk=>{output+=chunk});command.stderr.on('data',chunk=>{errors+=chunk});
      const timeout=setTimeout(()=>command.kill(),30000);
      try {await new Promise((resolve,reject)=>{command.once('error',reject);command.once('exit',code=>code===0?resolve():reject(Error(`轻量应用调用失败 ${code}: ${errors}`)))});}
      finally {clearTimeout(timeout);}
      const result=JSON.parse(output);assert.equal(result.status,'succeeded');
      assert.deepEqual(result.output,{connected:true,characters:400000,rootId:tree[0].id,undefinedPreserved:true,apiError:{code:'api_unavailable',effectState:'not_started',method:'omnitriggerMissing.noSuchMethod'}});
      assert(!(await call('tabs.query',[{}])).some(tab=>tab.url===base+'/light-script'));
      report.lightweight={host:option('--script-host'),nodeRequired:false,output:result.output};
      report.checks.push('真实轻量宿主 → 应用 Rust 客户端 → Chrome：书签、1 MiB Unicode、原生 undefined、结构化错误及测试标签页关闭后核验');
    }
  }
  report.ok = true;
} catch (error) {
  report.error = { code:error.code, message:error.message, stack:error.stack };
  process.exitCode = 1;
} finally {
  try {
    await context?.close();
    if (profile) {
      for (let attempt = 0; attempt < 20; attempt++) {
        if (!(await client.profiles()).some(item => item.browser_instance_id === profile)) {
          report.isolatedProfileDisconnected = true;
          break;
        }
        await delay(100);
      }
      assert(report.isolatedProfileDisconnected, '隔离 profile 尚未断开');
    }
  } catch (error) {
    report.cleanupError = { message: error.message };
    report.ok = false;
    process.exitCode = 1;
  } finally { client.close(); }
  server.close();
  await writeFile(join(scratch,'report.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify({...report, evidence:join(scratch,'report.json')}));
}
