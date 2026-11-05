import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const source = readFileSync(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js', import.meta.url), 'utf8');
const create = new Function(`${source}; return createDesktopApi;`)();

test('toggleHistory共享同步/异步业务，只接受无参和互斥创建/关闭回执', async () => {
  for (const asynchronous of [false, true]) {
    for (const result of [{opened:true}, {closed:true}]) {
      let calls = 0;
      const api = create((method, params) => {
        calls++;
        assert.equal(method, 'ui.toggle_history');
        assert.deepEqual(params, {});
        return asynchronous ? Promise.resolve(result) : result;
      });
      for (const extra of [undefined, null, {}, 'close']) {
        assert.throws(() => api.ui.toggleHistory(extra), error => error.code === 'invalid_input'
          && error.method === 'ui.toggle_history' && error.effectState === 'not_started');
      }
      assert.equal(calls, 0);
      const value = api.ui.toggleHistory();
      assert.equal(typeof value?.then === 'function', asynchronous);
      assert.deepEqual(await value, result);
      assert.equal(calls, 1);
    }
  }
});

test('非法或不确定回执不推断开关状态，也不重复调用', async () => {
  for (const asynchronous of [false, true]) {
    for (const result of [null, {}, [], {opened:false}, {closed:false}, {opened:true,closed:true},
      {closed:true,extra:1}, Object.assign(Object.create({closed:true}), {other:true})]) {
      let calls = 0;
      const api = create(() => { calls++; return asynchronous ? Promise.resolve(result) : result; });
      await assert.rejects(async () => await api.ui.toggleHistory(), error => error.code === 'invalid_response'
        && error.method === 'ui.toggle_history' && error.effectState === 'unknown');
      assert.equal(calls, 1);
    }
  }
});

test('busy/unavailable/unknown错误保持原样且不自动再切换', async () => {
  for (const asynchronous of [false, true]) {
    for (const [code, effectState] of [['ui_busy','not_started'], ['ui_unavailable','not_started'],
      ['timeout','unknown'], ['cancelled','unknown']]) {
      let calls = 0;
      const failure = Object.assign(new Error('合成错误'), {code, effectState, method:'ui.toggle_history'});
      const api = create(() => { calls++; if (asynchronous) return Promise.reject(failure); throw failure; });
      await assert.rejects(async () => await api.ui.toggleHistory(), error => error === failure);
      assert.equal(calls, 1);
    }
  }
});

test('显式toggle不改变openHistory只创建的原契约', async () => {
  for (const asynchronous of [false, true]) {
    const methods = [];
    const api = create(method => {
      methods.push(method);
      const value = method === 'ui.toggle_history' ? {closed:true} : {opened:true};
      return asynchronous ? Promise.resolve(value) : value;
    });
    assert.deepEqual(await api.ui.openHistory(), {opened:true});
    assert.deepEqual(await api.ui.toggleHistory(), {closed:true});
    assert.deepEqual(methods, ['ui.open_history', 'ui.toggle_history']);
  }
});
