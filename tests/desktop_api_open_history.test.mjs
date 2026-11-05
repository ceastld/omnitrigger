import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import open from '../examples/actions/clipboard-history/open.mjs';
import {main as openScript} from '../examples/actions/clipboard-history/open-script.mjs';

const source = readFileSync(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js', import.meta.url), 'utf8');
const create = new Function(`${source}; return createDesktopApi;`)();

test('openHistory两类宿主严格空参、创建回执与未知错误，不传目标或重试', async () => {
  for (const asyncCall of [false, true]) {
    let calls = 0;
    const api = create((method, params) => {
      calls++;
      assert.equal(method, 'ui.open_history');
      assert.deepEqual(params, {});
      return asyncCall ? Promise.resolve({opened:true}) : {opened:true};
    });
    assert.throws(() => api.ui.openHistory({target:'不能指定'}), error => error.code === 'invalid_input');
    assert.equal(calls, 0);
    assert.deepEqual(await api.ui.openHistory(), {opened:true});
    assert.equal(calls, 1);
  }
  let calls = 0;
  const bad = create(() => { calls++; return {opened:true, unexpected:1}; });
  assert.throws(() => bad.ui.openHistory(), error => error.code === 'invalid_response' && error.effectState === 'unknown');
  assert.equal(calls, 1);
});

test('两个打开动作直接调用同一原生API，不读取历史正文或另起主程序', async () => {
  for (const main of [open, openScript]) {
    const calls = [];
    const result = await main({ui:{openHistory(...args) { calls.push(args); return {opened:true}; }}});
    assert.deepEqual(result, {opened:true});
    assert.deepEqual(calls, [[]]);
  }
});
