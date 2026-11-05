// 只用合成数据和 RecordingHost；不启动真实宿主、窗口、监听器或系统剪贴板。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
import asyncMain from './main.mjs';
import {main as syncMain} from './main-script.mjs';
import {prepare, redact} from './workflow.mjs';

const id = '9007199254740993';
const createdAt = '2026-01-02T03:04:05.000Z';
const secret = '  合成正文 secret 文😀\n第二行\t ';
const entry = (key = id, value = secret) => ({id:key, preview:value.replace(/[\r\n]/g, ' ').slice(0, 64),
  utf16Units:value.length, createdAt});
const fault = (code = 'target_changed') => Object.assign(new Error(secret), {code, method:secret, effectState:'not_started'});

class RecordingHost {
  constructor(asynchronous, options = {}) {
    this.calls = [];
    this.inFlight = 0;
    this.peak = 0;
    const rows = options.rows ?? [entry()];
    const values = {
      'history.append':{id, revision:'18446744073709551615', inserted:true},
      'history.list':{items:rows, revision:'18446744073709551615'},
      'ui.choose':{status:'selected', id},
      'history.get':{id, text:secret, createdAt, revision:'18446744073709551615'},
      'keyboard.text':{submitted:true},
      ...options.values,
    };
    const invoke = (method, args) => {
      this.calls.push([method, args]);
      const perform = () => {
        if (method === options.failAt) throw options.error ?? fault();
        return structuredClone(values[method]);
      };
      if (!asynchronous) return perform();
      this.peak = Math.max(this.peak, ++this.inFlight);
      return Promise.resolve().then(perform).finally(() => this.inFlight--);
    };
    this.ctx = {clipboard:{history:{
      append: (...args) => invoke('history.append', args),
      list: (...args) => invoke('history.list', args),
      get: (...args) => invoke('history.get', args),
    }}, ui:{choose: (...args) => invoke('ui.choose', args)},
    desktop:{sendText: (...args) => invoke('keyboard.text', args)}};
  }
}

async function run(asynchronous, input, options) {
  const host = new RecordingHost(asynchronous, options);
  host.ctx.input = input;
  try {
    const output = asynchronous ? await asyncMain(host.ctx) : syncMain(host.ctx);
    assert.equal(output instanceof Promise, false);
    assert.equal(host.inFlight, 0);
    assert.ok(host.peak <= 1, 'Node 调用必须等待上一阶段完成');
    return {output, host};
  } catch (error) { return {error, host}; }
}

for (const asynchronous of [false, true]) {
  const name = asynchronous ? 'Node' : '同步';
  test(`${name} append 原样保存空文本、空白和正文，只返回摘要`, async () => {
    for (const text of ['', ' \t\r\n ', secret, '😀'.repeat(32768)]) {
      const {output, host} = await run(asynchronous, {mode:'append', text});
      assert.deepEqual(output, {operation:'append', inserted:true, utf16Units:text.length});
      assert.deepEqual(host.calls, [['history.append', [{text}]]]);
      assert.equal(JSON.stringify(output).includes(secret), false);
    }
    const {output} = await run(asynchronous, {mode:'append', text:secret},
      {values:{'history.append':{id, revision:'1', inserted:false}}});
    assert.equal(output.inserted, false);
  });

  test(`${name} 参数错误先于所有宿主访问`, async () => {
    const inputs = [null, [], {}, {mode:'copy'}, {mode:'append'}, {mode:'append',text:null},
      {mode:'append',text:'\ud800'}, {mode:'append',text:'\udfff'}, {mode:'append',text:'a\0b'},
      {mode:'append',text:'a'.repeat(65537)}, {mode:'append',text:'ok',limit:1},
      {mode:'list',text:'ok'}, {mode:'send',path:'file'}, {mode:'list',extra:true},
      ...[undefined, null, false, 0, 101, 1.5, '10'].map(limit => ({mode:'list',limit}))];
    for (const input of inputs) {
      const {error, host} = await run(asynchronous, input);
      assert.equal(error?.code, 'invalid_input');
      assert.equal(error.effectState, 'not_started');
      assert.deepEqual(host.calls, []);
    }
  });

  test(`${name} list 只显示摘要，不读取或发送正文`, async () => {
    const {output, host} = await run(asynchronous, {mode:'list',limit:1});
    assert.deepEqual(host.calls.map(row => row[0]), ['history.list', 'ui.choose']);
    assert.deepEqual(host.calls[0][1], [{limit:1}]);
    assert.deepEqual(output, {operation:'list',status:'selected',itemCount:1,
      selectedUtf16Units:secret.length,inputSubmitted:false});
    for (const value of [id, secret, createdAt, '18446744073709551615']) {
      assert.equal(JSON.stringify(output).includes(value), false);
    }
  });

  test(`${name} send 按稳定 ID 取单项，再向原目标发送一次原文`, async () => {
    const {output, host} = await run(asynchronous, {mode:'send'});
    assert.deepEqual(host.calls.map(row => row[0]), ['history.list', 'ui.choose', 'history.get', 'keyboard.text']);
    assert.deepEqual(host.calls[2][1], [{id}]);
    assert.deepEqual(host.calls[3][1], [secret]);
    assert.deepEqual(output, {operation:'send',status:'submitted',itemCount:1,
      selectedUtf16Units:secret.length,inputSubmitted:true});
  });

  test(`${name} 空历史、取消与空正文均不提交输入`, async () => {
    for (const mode of ['list', 'send']) {
      const empty = await run(asynchronous, {mode}, {rows:[]});
      assert.equal(empty.output.status, 'empty');
      assert.deepEqual(empty.host.calls.map(row => row[0]), ['history.list']);
      for (const reason of ['dismissed','timeout']) {
        const cancelled = await run(asynchronous, {mode}, {values:{'ui.choose':{status:'cancelled',reason}}});
        assert.equal(cancelled.output.status, 'cancelled');
        assert.deepEqual(cancelled.host.calls.map(row => row[0]), ['history.list','ui.choose']);
      }
    }
    const empty = await run(asynchronous, {mode:'send'}, {rows:[entry(id, '')],
      values:{'history.get':{id,text:'',createdAt,revision:'5'}}});
    assert.equal(empty.output.status, 'empty_text');
    assert.equal(empty.output.inputSubmitted, false);
    assert.deepEqual(empty.host.calls.map(row => row[0]), ['history.list','ui.choose','history.get']);
    assert.ok(empty.host.calls[1][1][0].items[0].label.includes('空白文本'));
  });

  test(`${name} 无效摘要和选择不能进入正文读取`, async () => {
    const badListings = [null, {items:[],revision:'01'}, {items:[],revision:'18446744073709551616'},
      {items:[entry(),entry()],revision:'0'}, {items:[{...entry(),id:'0'}],revision:'0'},
      {items:[{...entry(),preview:'\ud800'}],revision:'0'},
      {items:[{...entry(),preview:'😀'.repeat(33)}],revision:'0'},
      {items:[{...entry(),preview:'a\nb'}],revision:'0'},
      {items:[{...entry(),utf16Units:65537}],revision:'0'},
      {items:[{...entry(),text:secret}],revision:'0'}];
    for (const listing of badListings) {
      const {error, host} = await run(asynchronous, {mode:'send'}, {values:{'history.list':listing}});
      assert.equal(error?.code, 'invalid_result');
      assert.equal(host.calls.length, 1);
    }
    for (const choice of [null, {status:'selected',id:'2'}, {status:'selected',id, text:secret},
      {status:'cancelled',reason:'other'}]) {
      const {error, host} = await run(asynchronous, {mode:'send'}, {values:{'ui.choose':choice}});
      assert.equal(error?.code, 'invalid_result');
      assert.equal(host.calls.length, 2);
    }
  });

  test(`${name} 删除、取消、失焦及未知宿主错误不会触发回退或重试`, async () => {
    for (const [method, code, count] of [['history.list','cancelled',1], ['ui.choose','ui_busy',2],
      ['history.get','history_item_missing',3], ['keyboard.text','target_changed',4]]) {
      const {error, host} = await run(asynchronous, {mode:'send'}, {failAt:method,error:fault(code)});
      assert.equal(error.code, code);
      assert.equal(error.method, method);
      assert.equal(error.message.includes(secret), false);
      assert.equal(error.stack.includes(secret), false);
      assert.equal(host.calls.length, count);
    }
    const {error, host} = await run(asynchronous, {mode:'append',text:secret},
      {failAt:'history.append',error:new Error(secret)});
    assert.equal(error.code, 'history_action_failed');
    assert.equal(error.effectState, 'unknown');
    assert.equal(error.message.includes(secret), false);
    assert.equal(host.calls.length, 1);
  });

  test(`${name} 结果不一致拒绝发送，发送后的畸形结果保留未知效果`, async () => {
    for (const full of [{id:'2',text:secret,createdAt,revision:'1'},
      {id,text:'different',createdAt,revision:'1'}, {id,text:secret,createdAt:'changed',revision:'1'},
      {id,text:'\ud800',createdAt,revision:'1'}]) {
      const {error, host} = await run(asynchronous, {mode:'send'}, {values:{'history.get':full}});
      assert.equal(error?.code, 'invalid_result');
      assert.equal(host.calls.length, 3);
    }
    for (const mode of ['append','send']) {
      const method = mode === 'append' ? 'history.append' : 'keyboard.text';
      const {error, host} = await run(asynchronous, {mode,...(mode === 'append' ? {text:secret} : {})},
        {values:{[method]:{}}});
      assert.equal(error.code, 'invalid_result');
      assert.equal(error.effectState, 'unknown');
      assert.equal(host.calls.filter(call => call[0] === method).length, 1);
    }
  });
}

test('最坏摘要数量仍在 UI 请求大小上限内，标签替换不影响正文', async () => {
  const rows = Array.from({length:100}, (_, index) => entry(String(index + 1), '中'.repeat(64)));
  const {host} = await run(true, {mode:'list'}, {rows, values:{'ui.choose':{status:'selected',id:'1'}}});
  const params = host.calls[1][1][0];
  assert.ok(Buffer.byteLength(JSON.stringify(params), 'utf8') <= 32 * 1024);
  const result = await run(false, {mode:'list'}, {rows:[{...entry(),preview:'\t\u0001  '}]});
  assert.ok(result.host.calls[1][1][0].items[0].label.includes('空白文本'));
});

test('缺少历史服务明确失败，不回退系统剪贴板', async () => {
  for (const [main, asynchronous] of [[syncMain,false], [asyncMain,true]]) {
    const ctx = {input:{mode:'append',text:'safe'},clipboard:{
      readText: () => {throw Error('禁止回退');}, readData: () => {throw Error('禁止回退');},
    }};
    let error;
    try { if (asynchronous) await main(ctx); else main(ctx); } catch (value) { error = value; }
    assert.equal(error?.code, 'history_unavailable');
    assert.equal(error.effectState, 'not_started');
  }
});

test('清单中的管理和打开入口与调用信封一致，管理默认仅展示摘要', () => {
  const manifest = JSON.parse(readFileSync(new URL('action.json', import.meta.url), 'utf8'));
  const expected = {open:'node','open-script':'script',manage:'node','manage-script':'script'};
  assert.equal(manifest.actions.length, Object.keys(expected).length);
  for (const action of manifest.actions) assert.equal(action.runtime, expected[action.id]);
  for (const name of ['call.json','call-script.json','append-call.json','send-call.json','open-call.json','open-script-call.json']) {
    const value = JSON.parse(readFileSync(new URL(name, import.meta.url), 'utf8'));
    assert.equal(value.package_id, manifest.id);
    const action = manifest.actions.find(action => action.id === value.action_id);
    assert.equal(action.version, value.action_version);
    assert.equal(action.single_instance, true);
    if (name === 'call.json' || name === 'call-script.json') assert.equal(value.input.mode, 'list');
  }
});

test('两个打开入口只提交原生历史窗请求，保留未知结果且不重试', async () => {
  for (const name of ['open.mjs','open-script.mjs']) {
    const module = await import(new URL(name, import.meta.url));
    const main = module.default ?? module.main;
    let calls = 0;
    assert.deepEqual(await main({ui:{openHistory(){calls++;return {opened:true};}}}), {opened:true});
    assert.equal(calls,1);
    const failure = Object.assign(new Error('合成未知结果'),{code:'timeout',effectState:'unknown'});
    try { await main({ui:{openHistory(){calls++;throw failure;}}}); assert.fail('未知结果应传回调用者'); }
    catch (error) { assert.equal(error,failure); }
    assert.equal(calls,2);
  }
});

test('独立 history 工厂和现有桌面 SDK 可在两入口串行组合，仍只有合成桥接', async () => {
  const runtime = new URL('../../../crates/omnitrigger-actions/runtime/', import.meta.url);
  const historyFactory = runInNewContext(readFileSync(new URL('clipboard-history-api.js', runtime), 'utf8') + ';createClipboardHistoryApi');
  const desktopFactory = runInNewContext(readFileSync(new URL('desktop-api.js', runtime), 'utf8') + ';createDesktopApi');
  for (const asynchronous of [false, true]) {
    const calls = [];
    const bridge = (method, params) => {
      calls.push([method, params]);
      const values = {
        'history.list':{items:[entry()],revision:'0'},
        'history.get':{id,text:secret,createdAt,revision:'1'},
        'ui.choose':{status:'selected',id},
        'keyboard.text':{submitted:true},
      };
      return asynchronous ? Promise.resolve(values[method]) : values[method];
    };
    const api = desktopFactory(bridge);
    const ctx = {input:{mode:'send'}, ...api,
      clipboard:{...api.clipboard,history:historyFactory(bridge)}};
    const result = asynchronous ? await asyncMain(ctx) : syncMain(ctx);
    assert.equal(result.inputSubmitted, true);
    assert.deepEqual(calls.map(call => call[0]), ['history.list','ui.choose','history.get','keyboard.text']);
    assert.deepEqual(JSON.parse(JSON.stringify(calls[3][1])), {text:secret});
  }
});

test('输入在首个宿主阶段前固定，不受随后对象变化影响', () => {
  const input = {mode:'list',limit:1};
  const task = prepare(input);
  input.mode = 'send';
  const choice = task.resume({items:[entry()],revision:'0'});
  assert.equal(choice.resume({status:'selected',id}).value.operation, 'list');
  const append = {mode:'append',text:secret};
  const save = prepare(append);
  append.text = 'changed';
  assert.equal(save.args[0].text, secret);
  assert.equal(save.resume({id,revision:'1',inserted:true}).value.utf16Units, secret.length);
});

test('异常 getter 不能将正文作为错误消息或字段泄漏', () => {
  const hostile = Object.defineProperty({}, 'code', {get: () => {throw Error(secret);}});
  const error = redact(hostile, 'history.append', true);
  assert.equal(error.code, 'history_action_failed');
  assert.equal(error.effectState, 'unknown');
  assert.equal(error.message.includes(secret), false);
  assert.equal(error.stack.includes(secret), false);
});
