import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const context = vm.createContext({});
vm.runInContext(readFileSync(new URL('./system-panel.js', import.meta.url), 'utf8'), context);
const run = context.main;
const plain = value => JSON.parse(JSON.stringify(value));

test('固定发送一次左 Win+A，由原生 API 按原目标解析时序', () => {
  const calls = [];
  const result = run({input: {}, desktop: {
    sendShortcut(keys, options) { calls.push([plain(keys), plain(options)]); return {submitted: true}; },
  }});
  assert.deepEqual(calls, [[[91, 65], {timing: {key_down_delay_ms: 0,
    process_overrides: {wps: 10, et: 10, wpp: 10, wpspdf: 10, mstsc: 10}}}]]);
  assert.deepEqual(plain(result), {inputSubmitted: true});
});

test('非空及非对象参数在发送前拒绝', () => {
  for (const input of [{keys: [91, 78]}, [], null, undefined, 'Win+A']) {
    let sent = 0;
    assert.throws(() => run({input, desktop: {sendShortcut() { sent++; }}}),
      error => error.code === 'invalid_input' && error.effectState === 'not_started');
    assert.equal(sent, 0);
  }
});

test('原生错误直接传播，不重试输入或更换组合键', () => {
  const failure = new Error('fixture-original-target-changed');
  let sent = 0;
  assert.throws(() => run({input: {}, desktop: {sendShortcut() { sent++; throw failure; }}}), error => error === failure);
  assert.equal(sent, 1);
});

test('回执不明确时保留效果未知，不重试', () => {
  for (const receipt of [undefined, null, {}, {submitted: false}]) {
    let sent = 0;
    assert.throws(() => run({input: {}, desktop: {sendShortcut() { sent++; return receipt; }}}),
      error => error.code === 'input_unconfirmed' && error.effectState === 'unknown');
    assert.equal(sent, 1);
  }
});
