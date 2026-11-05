import assert from 'node:assert/strict';
import {readFileSync, writeFileSync, mkdirSync, mkdtempSync, rmSync, realpathSync} from 'node:fs';
import {resolve, join, dirname, relative, isAbsolute} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createServer} from 'node:net';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {runInNewContext} from 'node:vm';
import test from 'node:test';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const runtime = resolve(repo, 'crates/omnitrigger-actions/runtime');
const desktopSource = readFileSync(join(runtime, 'desktop-api.js'), 'utf8');
const browserSource = readFileSync(join(runtime, 'browser-api.js'), 'utf8');
const hostSource = readFileSync(join(runtime, 'host.mjs'), 'utf8');
const sdkSource = readFileSync(join(repo, 'tools/js-engine-probe/src/sdk.js'), 'utf8');
const createApi = runInNewContext(desktopSource + ';createDesktopApi');
const token = 'b'.repeat(64); // 独立测试回环连接的虚构凭据，不读取个人配置。
const defaults = message => ({message,title:'OmniTrigger',level:'info',placement:'topRight',durationMs:3000});
const invalidFault = error => error.code === 'invalid_input' && error.effectState === 'not_started' && error.method === 'ui.notify';
const plain = value => JSON.parse(JSON.stringify(value));

function invalidOptions() {
  const invalid = [undefined,null,[],42,true,'text',{}, {message:null},{message:1},{message:''},{message:' \r\n\t '},
    {message:'x'.repeat(1025)},{message:'😀'.repeat(513)}, {message:'\ud800'},{message:'\udfff'},
    {message:'\ud800x'},{message:'x\udc00'}, {message:'a',title:null},{message:'a',title:''},{message:'a',title:'  '},
    {message:'a',title:42},{message:'a',title:'x'.repeat(129)},{message:'a',title:'😀'.repeat(65)},
    {message:'a',title:'A\u2028B'},{message:'a',title:'A\u2029B'},{message:'a',title:'\ud800'},
    {message:'a',level:'Info'},{message:'a',level:null},{message:'a',level:1},
    {message:'a',placement:'topLeft'},{message:'a',placement:null},{message:'a',placement:1},
    {message:'a',durationMs:999},{message:'a',durationMs:30001},{message:'a',durationMs:1000.5},
    {message:'a',durationMs:'3000'},{message:'a',durationMs:null},{message:'a',durationMs:NaN},
    {message:'a',durationMs:Infinity},{message:'a',extra:true}, {message:'a',html:true},
    {message:'a',[Symbol('extra')]:true}, Object.defineProperty({message:'a'},'extra',{value:true})];
  for (let code = 0; code <= 0x9f; code++) {
    if ((code < 0x20 || code >= 0x7f) && ![9,10,13].includes(code)) invalid.push({message:`a${String.fromCharCode(code)}b`});
    if (code < 0x20 || code >= 0x7f) invalid.push({message:'a',title:`a${String.fromCharCode(code)}b`});
  }
  for (const whitespace of ['\u00a0','\u1680','\u2000','\u200a','\u2028','\u2029','\u202f','\u205f','\u3000']) {
    invalid.push({message:whitespace}, {message:'a',title:whitespace});
  }
  return invalid;
}

test('共享通知契约：严格参数与 Unicode 边界在调用宿主前校验', () => {
  const calls = [];
  const ui = createApi((method, params) => {calls.push([method,plain(params)]); return {submitted:true};}).ui;
  const message = '  <b>纯文本</b>\r\n\t😀  ';
  assert.deepEqual(ui.notify({message}), {submitted:true});
  assert.deepEqual(calls.pop(), ['ui.notify',defaults(message)]);
  for (const options of invalidOptions()) assert.throws(() => ui.notify(options), invalidFault);
  assert.equal(calls.length, 0, '无效通知不能到达宿主');
  for (const options of [
    {message:'x'.repeat(1024),title:'x'.repeat(128),durationMs:1000},
    {message:'😀'.repeat(512),title:'😀'.repeat(64),durationMs:30000},
    {message:'a\u2028b\u2029c'},
    {message:'\ufeff',title:'\ufeff'},
    ...['info','success','warning','error'].map(level => ({message:'a',level})),
    ...['topRight','bottomRight','center'].map(placement => ({message:'a',placement})),
  ]) assert.deepEqual(ui.notify(options), {submitted:true});
});

function light(native, reply = {ok:true,value:{submitted:true}}) {
  const calls = [];
  const context = {ctx:native === undefined ? {} : {__nativeContext:native}, __hostCall: serialized => {
    calls.push(JSON.parse(serialized));
    return JSON.stringify(reply);
  }};
  runInNewContext(desktopSource + browserSource + sdkSource, context);
  return {ctx:context.ctx,calls};
}

test('轻量同步 SDK 复用参数并隐藏桥接凭据；无 UI 明确返回 ui_unavailable', () => {
  const fixture = light({connection:{port:1234,token}});
  const result = fixture.ctx.ui.notify({message:'hello',placement:'center',durationMs:30000});
  assert.equal(typeof result?.then, 'undefined');
  assert.deepEqual(plain(result), {submitted:true});
  assert.equal(fixture.calls[0].kind, 'native');
  assert.equal(fixture.calls[0].method, 'ui.notify');
  assert.deepEqual(fixture.calls[0].params, {...defaults('hello'),placement:'center',durationMs:30000});
  assert.equal(fixture.ctx.__nativeContext, undefined);
  assert.equal(JSON.stringify(fixture.ctx).includes(token), false);
  for (const native of [undefined,{error:{code:'desktop_unavailable',message:'UI 未运行',effect_state:'not_started'}}]) {
    const fixture = light(native);
    assert.throws(() => fixture.ctx.ui.notify({message:'hello'}), error => error.code === 'ui_unavailable' && error.method === 'ui.notify' && error.effectState === 'not_started' && !String(error).includes(token));
    assert.equal(fixture.calls.length, 0);
  }
  const invalid = light({connection:{port:1234,token}});
  for (const options of invalidOptions()) assert.throws(() => invalid.ctx.ui.notify(options), invalidFault);
  assert.equal(invalid.calls.length, 0);
});

test('轻量 SDK 保留结构化原生错误字段', () => {
  for (const [code,effect] of [['ui_busy','not_started'],['ui_unavailable','not_started'],['cancelled','not_started'],['timeout','unknown']]) {
    const fixture = light({connection:{port:1234,token}}, {ok:false,error:{code,message:'未确认',effect_state:effect,method:'ui.notify',required_permissions:['test.permission']}});
    assert.throws(() => fixture.ctx.ui.notify({message:'hello'}), error => error.code === code && error.method === 'ui.notify' && error.effectState === effect && error.requiredPermissions[0] === 'test.permission' && !String(error).includes(token));
  }
});

async function nodeInvocation(source, {handle, native = true, cancel = false, nativeError} = {}) {
  const scratch = resolve(repo, '.tmp/parallel-notify/sdk');
  mkdirSync(scratch, {recursive:true});
  const directory = mkdtempSync(join(scratch, 'node-'));
  const sockets = new Set();
  const errors = [];
  let child;
  const server = createServer(socket => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
    let input = '';
    let handled = false;
    socket.setEncoding('utf8');
    socket.on('data', chunk => {
      input += chunk;
      if (handled || !input.endsWith('\n')) return;
      handled = true;
      try {
        const frame = JSON.parse(input);
        assert.equal(frame.token, token);
        assert.equal(frame.method, 'ui.notify');
        handle?.(frame, socket);
        if (cancel) child.stdin.write('cancel\n');
      } catch (error) {errors.push(error); socket.destroy();}
    });
  });
  let timer;
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const entry = join(directory, 'entry.mjs');
    const host = join(directory, 'host.mjs');
    const response = join(directory, 'response.json');
    writeFileSync(entry, source);
    writeFileSync(host, desktopSource + browserSource + hostSource);
    child = spawn(process.execPath, [host], {stdio:['pipe','pipe','pipe'],windowsHide:true});
    let stderr = '';
    child.stderr.on('data', chunk => {stderr += chunk;});
    const exited = once(child, 'exit');
    timer = setTimeout(() => child.kill(), 6000);
    child.stdin.write(JSON.stringify({api_version:1,invocation:{invocation_id:'1',binding_id:'2'},input:{},
      entry_path:entry,package_root:directory,response_path:response,timeout_ms:4000,
      ...(native ? {native:{port:server.address().port,token}} : {}), ...(nativeError ? {native_error:nativeError} : {})}) + '\n');
    const [code] = await exited;
    assert.equal(code, 0, stderr);
    assert.deepEqual(errors, []);
    const result = JSON.parse(readFileSync(response, 'utf8'));
    assert.equal(JSON.stringify(result).includes(token), false, '回执不能包含桥接凭据');
    return result;
  } finally {
    clearTimeout(timer);
    if (child && child.exitCode === null) child.kill();
    for (const socket of sockets) socket.destroy();
    await new Promise(resolve => server.close(resolve));
    const target = realpathSync(directory);
    const boundary = realpathSync(scratch);
    const inside = relative(boundary, target);
    assert(inside && !inside.startsWith('..') && !isAbsolute(inside));
    rmSync(target, {recursive:true,force:true});
  }
}

test('Node notify 返回 Promise，只等创建回执而不等待 30 秒显示结束', {timeout:8000}, async () => {
  let calls = 0;
  const began = Date.now();
  const result = await nodeInvocation(`export default async ctx => {
    const value = ctx.ui.notify({message:'plain <b>text</b>\\nnext',level:'success',placement:'bottomRight',durationMs:30000});
    return {promise:typeof value.then==='function',receipt:await value,hidden:ctx.__nativeContext===undefined};
  }`, {handle:(frame,socket) => {
    calls++;
    assert.deepEqual(frame.params, {...defaults('plain <b>text</b>\nnext'),level:'success',placement:'bottomRight',durationMs:30000});
    socket.end(JSON.stringify({id:frame.id,ok:true,value:{submitted:true}})+'\n');
  }});
  assert.equal(result.status, 'ok');
  assert.deepEqual(result.output, {promise:true,receipt:{submitted:true},hidden:true});
  assert.equal(calls, 1);
  assert(Date.now()-began < 5000, '通知回执不能等待 durationMs');
});

test('Node 无 UI 及参数错误使用异步拒绝并保留 ui.notify 方法', {timeout:12000}, async () => {
  const source = `export default async ctx => {
    const value=ctx.ui.notify({message:'test'});
    try {await value;} catch(e) {return {promise:typeof value.then==='function',code:e.code,effect:e.effectState,method:e.method};}
  }`;
  for (const nativeError of [undefined,{code:'desktop_unavailable',message:'UI 未运行',effect_state:'not_started'}]) {
    const result = await nodeInvocation(source, {native:false,nativeError});
    assert.deepEqual(result.output, {promise:true,code:'ui_unavailable',effect:'not_started',method:'ui.notify'});
  }
  const result = await nodeInvocation(`export default async ctx => {
    const value=ctx.ui.notify({message:'ok',durationMs:0});
    try {await value;} catch(e) {return {promise:typeof value.then==='function',code:e.code,effect:e.effectState,method:e.method};}
  }`, {native:false});
  assert.deepEqual(result.output, {promise:true,code:'invalid_input',effect:'not_started',method:'ui.notify'});
});

test('Node 原生错误字段与通知取消不会丢失方法或凭据外泄', {timeout:12000}, async () => {
  for (const [code,effect] of [['ui_busy','not_started'],['ui_unavailable','not_started'],['cancelled','not_started'],['timeout','unknown']]) {
    const result = await nodeInvocation(`export default async ctx => await ctx.ui.notify({message:'test'});`, {handle:(frame,socket) => {
      socket.end(JSON.stringify({id:frame.id,ok:false,error:{code,message:'未确认',effect_state:effect,method:'ui.notify',required_permissions:['test.permission']}})+'\n');
    }});
    assert.equal(result.status, 'error');
    assert.deepEqual(result.error, {code,message:'未确认',effect_state:effect,method:'ui.notify',required_permissions:['test.permission']});
  }
  const cancelled = await nodeInvocation(`export default async ctx => await ctx.ui.notify({message:'test'});`, {cancel:true});
  assert.equal(cancelled.error.code, 'cancelled');
  assert.equal(cancelled.error.method, 'ui.notify');
  assert.equal(cancelled.error.effect_state, 'unknown');
});

test('同一 Node 调用内的重叠通知明确 ui_busy，不提交第二个请求', {timeout:8000}, async () => {
  let calls = 0;
  const result = await nodeInvocation(`export default async ctx => {
    const first=ctx.ui.notify({message:'first'});
    let overlap;
    try {await ctx.ui.notify({message:'second'});} catch(e) {overlap={code:e.code,method:e.method,effect:e.effectState};}
    return {first:await first,overlap};
  }`, {handle:(frame,socket) => {
    calls++;
    socket.end(JSON.stringify({id:frame.id,ok:true,value:{submitted:true}})+'\n');
  }});
  assert.equal(calls,1);
  assert.deepEqual(result.output,{first:{submitted:true},overlap:{code:'ui_busy',method:'ui.notify',effect:'not_started'}});
});

test('通用示例的两个入口只转交通知参数并返回回执', async () => {
  const example = resolve(repo, 'examples/actions/script-notification');
  const input = {message:'通用通知',title:'测试',level:'warning',placement:'center',durationMs:1000};
  const calls = [];
  const ui = createApi((method, params) => {calls.push([method,plain(params)]); return {submitted:true};}).ui;
  const main = runInNewContext(readFileSync(join(example, 'main.js'),'utf8')+';main');
  assert.deepEqual(main({input,ui}), {submitted:true});
  const node = (await import(new URL('../examples/actions/script-notification/main.mjs', import.meta.url))).default;
  assert.deepEqual(await node({input,ui:{notify:async options => ui.notify(options)}}), {submitted:true});
  assert.deepEqual(calls, [['ui.notify',input],['ui.notify',input]]);
});
