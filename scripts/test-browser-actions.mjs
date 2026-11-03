// 真实 Chrome 验收：复用已注册的 Native Host，仅操作本次创建的隔离 profile。
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { resolve, join, relative } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { setTimeout as delay } from 'node:timers/promises';

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
const { connect } = await import(pathToFileURL(join(connector, 'sdk-js/index.mjs')));
const config = JSON.parse(await readFile(join(connector, 'connection.json'), 'utf8'));
const client = await connect({ pipe: config.pipe, timeoutMs: 3000 });
let context;
const server = createServer((request, response) => { response.setHeader('Content-Type', 'text/html; charset=utf-8'); response.end(`<title>OmniTrigger ${request.url}</title><h1>浏览器动作测试</h1><button type="button">测试交互</button>`); });
try {
  const existing = await client.profiles();
  report.existingProfiles = existing.length;
  context = await playwright.chromium.launchPersistentContext(join(scratch, 'profile'), {
    executablePath: chrome, headless: true, ignoreDefaultArgs: ['--disable-extensions'],
    args: ['--enable-unsafe-extension-debugging', '--no-first-run', '--no-default-browser-check'],
  });
  const cdp = await context.browser().newBrowserCDPSession();
  const loaded = await cdp.send('Extensions.loadUnpacked', { path: extension });
  report.chromeVersion = context.browser().version();
  let worker = context.serviceWorkers().find(item => item.url().startsWith(`chrome-extension://${loaded.id}/`));
  worker ??= await context.waitForEvent('serviceworker', { timeout: 15000 });
  // 首次 CDP 加载有时尚未派发安装事件；从扩展自己的状态入口确认并恢复连接。
  const popup = await context.newPage();
  await popup.goto(`chrome-extension://${loaded.id}/popup.html`);
  await popup.evaluate(() => chrome.runtime.sendMessage({type:'bridge.enable', enabled:true}));
  await popup.close();
  let profile;
  for (let attempt = 0; attempt < 100; attempt++) {
    const identity = await worker.evaluate(() => chrome.storage.local.get('browserInstanceId'));
    profile = identity.browserInstanceId;
    if (profile && (await client.profiles()).some(item => item.browser_instance_id === profile)) break;
    await delay(100);
  }
  assert(profile && !existing.some(item => item.browser_instance_id === profile), '必须唯一定位新建的隔离 profile');
  report.connectionStatus = await worker.evaluate(() => chrome.storage.session.get('bridgeStatus'));
  assert((await client.profiles()).some(item => item.browser_instance_id === profile), '扩展尚未连接');
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
  await assert.rejects(call('omnitriggerMissing.noSuchMethod'), error => { report.apiError = {code:error.code, effectState:error.effectState}; return !!error.code; });
  await assert.rejects(call('tabs.get', [2147483647]), error => error.code === 'chrome_error');
  const cancellation = new AbortController(); cancellation.abort();
  const count = (await call('tabs.query', [{}])).length;
  await assert.rejects(call('tabs.create', [{url:base + '/cancelled'}], {signal:cancellation.signal}), error => error.code === 'cancelled' && error.effectState === 'not_started');
  assert.equal((await call('tabs.query', [{}])).length, count);
  report.checks.push('未知 API、无效标签页、派发前取消且没有新建标签页');
  // 在隔离 profile 中放置合成大结果，验证 SDK 分块及脚本内处理，最终只返回摘要。
  await worker.evaluate(() => chrome.storage.local.set({omnitriggerActionFixture: '中文🦀'.repeat(100000)}));
  const host = await readFile(join(root, 'crates/omnitrigger-actions/runtime/host.mjs'), 'utf8');
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
    for (const file of ['sdk-js/index.mjs','sdk-js/protocol.mjs','connection.json']) {
      await writeFile(join(runtime,file),await readFile(join(connector,file)));
    }
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
  }
  report.ok = true;
} catch (error) {
  report.error = { code:error.code, message:error.message, stack:error.stack };
  process.exitCode = 1;
} finally {
  client.close();
  await context?.close();
  server.close();
  await writeFile(join(scratch,'report.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify({...report, evidence:join(scratch,'report.json')}));
}
