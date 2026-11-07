import assert from 'node:assert/strict';
import {readFileSync, mkdtempSync, writeFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createServer} from 'node:net';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {runInNewContext} from 'node:vm';
import test from 'node:test';

const runtime = new URL('../../../crates/omnitrigger-actions/runtime/', import.meta.url);
const desktopSource = readFileSync(new URL('desktop-api.js', runtime), 'utf8');
const browserSource = readFileSync(new URL('browser-api.js', runtime), 'utf8');
const historySource = readFileSync(new URL('clipboard-history-api.js', runtime), 'utf8');
const captureSource = readFileSync(new URL('process-capture-api.js', runtime), 'utf8');
const actionsSource = readFileSync(new URL('actions-api.js', runtime), 'utf8');
const hostSource = readFileSync(new URL('host.mjs', runtime), 'utf8');
const sdkSource = readFileSync(new URL('../../../tools/js-engine-probe/src/sdk.js', import.meta.url), 'utf8');
const api = runInNewContext(desktopSource + ';createDesktopApi');
const valid = () => ({items:[{id:'stable-a',label:'甲'},{id:'stable-b',label:'乙',description:'搜索说明'}]});

test('共享选择契约校验、稳定 ID 和中文参数', () => {
  let calls = 0;
  const ui = api((method, params) => {
    calls++;
    assert.equal(method, 'ui.choose');
    assert.equal(params.items[1].id, 'stable-b');
    return {status:'selected', id:'stable-b'};
  }).ui;
  assert.deepEqual(ui.choose(valid()), {status:'selected',id:'stable-b'});
  assert.equal(calls, 1);
  const invalid = [null, [], {}, {...valid(),title:null}, {...valid(),title:''},
    {...valid(),placeholder:5}, {...valid(),timeoutMs:0}, {...valid(),timeoutMs:60001},
    {...valid(),timeoutMs:1.5}, {...valid(),timeoutMs:null}, {...valid(),extra:1},
    {items:[]}, {items:[{id:'same',label:'A'},{id:'same',label:'B'}]},
    {items:[{id:'a',label:'A',value:1}]}, {items:[{id:'a',label:'A',description:null}]},
    {items:[{id:' ',label:'A'}]}, {items:[{id:'a',label:'A\nB'}]},
    {items:[{id:'a',label:'\ud800'}]}, {items:[{id:'a',label:'A\u2028B'}]},
    {items:[{id:'😀'.repeat(65),label:'A'}]},
    {items:Array.from({length:201}, (_,i) => ({id:String(i),label:'A'}))},
    {items:Array.from({length:100}, (_,i) => ({id:String(i),label:'中'.repeat(200)}))}];
  for (const options of invalid) assert.throws(() => ui.choose(options), error => error.code === 'invalid_input' && error.effectState === 'not_started' && error.method === 'ui.choose');
  assert.equal(calls, 1, '无效参数不能进入宿主');
});

test('轻量 SDK 使用相同参数与结果；独立 CLI 明确拒绝 UI', () => {
  let request;
  const context = {ctx:{__nativeContext:{connection:{port:1234,token:'a'.repeat(64)}}},
    __hostCall: source => {
      request = JSON.parse(source);
      return JSON.stringify({ok:true,value:{status:'cancelled',reason:'dismissed'}});
    }};
  runInNewContext(desktopSource + browserSource + historySource + captureSource + actionsSource + sdkSource, context);
  assert.equal(context.ctx.ui.choose(valid()).reason, 'dismissed');
  assert.equal(request.kind, 'native');
  assert.equal(request.method, 'ui.choose');
  assert.deepEqual(request.params, valid());
  const cli = {ctx:{},__hostCall: () => {throw Error('不应调用独立 CLI 的本机桥接');}};
  runInNewContext(desktopSource + browserSource + historySource + captureSource + actionsSource + sdkSource, cli);
  assert.throws(() => cli.ctx.ui.choose(valid()), error => error.code === 'ui_unavailable' && error.method === 'ui.choose');
});

async function nodeInvocation(source, handle, {cancel = false} = {}) {
  const directory = mkdtempSync(join(tmpdir(), 'omnitrigger-ui-contract-'));
  const sockets = new Set();
  const server = createServer(socket => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    let input = '';
    socket.setEncoding('utf8');
    socket.on('data', chunk => {
      input += chunk;
      if (!input.endsWith('\n')) return;
      const request = JSON.parse(input);
      handle(request, socket);
      if (cancel) child.stdin.write('cancel\n');
    });
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  let child;
  try {
    const entry = join(directory, 'entry.mjs');
    const host = join(directory, 'host.mjs');
    const response = join(directory, 'response.json');
    writeFileSync(entry, source);
    writeFileSync(host, desktopSource + browserSource + historySource + captureSource + actionsSource + hostSource);
    child = spawn(process.execPath, [host], {stdio:['pipe','pipe','pipe'],windowsHide:true});
    let stderr = '';
    child.stderr.on('data', chunk => { stderr += chunk; });
    const exited = once(child, 'exit');
    child.stdin.write(JSON.stringify({api_version:1,invocation:{invocation_id:'1',binding_id:'1'},input:{},
      entry_path:entry,package_root:directory,response_path:response,timeout_ms:10_000,
      native:{port:server.address().port,token:'a'.repeat(64)}}) + '\n');
    const [code] = await exited;
    assert.equal(code, 0, stderr);
    return JSON.parse(readFileSync(response, 'utf8'));
  } finally {
    child?.kill();
    for (const socket of sockets) socket.destroy();
    await new Promise(resolve => server.close(resolve));
    rmSync(directory, {recursive:true,force:true});
  }
}

test('Node await 选择可超过旧的五秒网络时限', {timeout:15_000}, async () => {
  let timer;
  try {
    const result = await nodeInvocation(`export default async ctx => await ctx.ui.choose(${JSON.stringify(valid())});`, (request, socket) => {
      assert.equal(request.method, 'ui.choose');
      timer = setTimeout(() => socket.end(JSON.stringify({id:request.id,ok:true,value:{status:'selected',id:'stable-b'}}) + '\n'), 5200);
    });
    assert.equal(result.status, 'ok');
    assert.deepEqual(result.output, {status:'selected',id:'stable-b'});
  } finally { clearTimeout(timer); }
});

test('Node 动作取消会断开未完成选择连接', {timeout:5000}, async () => {
  let disconnected;
  const result = await nodeInvocation(`export default async ctx => ctx.ui.choose(${JSON.stringify(valid())});`, (_, socket) => {
    disconnected = once(socket, 'close');
  }, {cancel:true});
  assert.equal(result.status, 'error');
  assert.equal(result.error.code, 'cancelled');
  assert.ok(disconnected, '选择调用已到达本机桥接');
  await disconnected;
});

test('Node 遗留选择 Promise 拒绝结束并关闭连接', {timeout:5000}, async () => {
  const result = await nodeInvocation(`export default ctx => {ctx.ui.choose(${JSON.stringify(valid())});return null;}`, () => {});
  assert.equal(result.status, 'error');
  assert.equal(result.error.code, 'pending_desktop_calls');
});
