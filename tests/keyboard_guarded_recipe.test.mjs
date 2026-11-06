import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const root = new URL('../examples/actions/keyboard-sequence/', import.meta.url);
const manifest = JSON.parse(await readFile(new URL('action.json', root), 'utf8'));
const action = manifest.actions.find(action => action.id === 'guarded-recipe-script');
assert.equal(action.runtime, 'script');
// 从清单加载入口，防止误接到允许省略名单的普通配方。
const {main} = await import(new URL(action.entry, root));
const sequence = {kind:'sequence', modifiers:[162], keys:[75, 67]};
function fixture(input, {processName = 'Code.EXE', onTarget, targetError, inputError} = {}) {
  const calls = [];
  const ctx = {input:structuredClone(input), desktop:{
    target() {
      calls.push(['target']); onTarget?.(ctx);
      if (targetError) throw targetError;
      return {processName};
    },
    sendKeySequence(modifiers, keys, options) {
      calls.push(structuredClone(['sequence', modifiers, keys, options]));
      if (inputError) throw inputError;
    },
    sendText(text) { calls.push(['text', text]); },
  }, delay(milliseconds) { calls.push(['delay', milliseconds]); }};
  return {ctx, calls};
}
const invalid = error => error.code === 'invalid_input' && error.effectState === 'not_started';

test('清单入口要求明确名单，漏传或无效名单时零宿主调用', () => {
  for (const allowedProcesses of [undefined, null, [], ['code', 'CODE'], ['code.exe'],
    ['a/b'], ['a\\b'], ['.'], ['..'], [''], ['a'.repeat(65)], [1],
    ['编辑器'], Array(17).fill('code')]) {
    const f = fixture({steps:[sequence], keyDownDelayMs:0, allowedProcesses});
    assert.throws(() => main(f.ctx), invalid);
    assert.deepEqual(f.calls, []);
  }
  for (const input of [undefined, null, {}, []]) {
    const f = fixture(input);
    assert.throws(() => main(f.ctx), invalid);
    assert.deepEqual(f.calls, []);
  }
});

test('显式延时也查询原目标，名单不匹配时零输入与等待', () => {
  for (const keyDownDelayMs of [undefined, 0, 10]) {
    const f = fixture({allowedProcesses:['devenv'], keyDownDelayMs,
      steps:[{kind:'delay', milliseconds:10}, sequence]});
    assert.throws(() => main(f.ctx), error => error.code === 'target_process_not_allowed'
      && error.effectState === 'not_started' && error.method === 'desktop.target');
    assert.deepEqual(f.calls, [['target']]);
  }
});

test('按大小写无关的进程基本名匹配，保留顺序和显式零延时', () => {
  const f = fixture({allowedProcesses:['CODE'], keyDownDelayMs:0, steps:[sequence,
    {kind:'delay', milliseconds:30}, {kind:'tap', key:8}, {kind:'text', text:'中文😀'}]});
  assert.deepEqual(main(f.ctx), {inputSubmitted:true, steps:4});
  assert.deepEqual(f.calls, [['target'], ['sequence', [162], [75, 67], {keyDownDelayMs:0}],
    ['delay', 30], ['sequence', [], [8], {keyDownDelayMs:0}], ['text', '中文😀']]);
});

test('WPS 与远程桌面的默认延时保留，其他进程默认零', () => {
  for (const processName of ['wps.exe', 'ET', 'wpp', 'wpspdf', 'mstsc', 'code']) {
    const f = fixture({allowedProcesses:[processName.replace(/\.exe$/, '')], steps:[sequence]}, {processName});
    main(f.ctx);
    assert.equal(f.calls[1][3].keyDownDelayMs, processName === 'code' ? 0 : 10);
    assert.equal(f.calls.filter(call => call[0] === 'target').length, 1);
  }
});

test('31 步加目标查询可执行，32 步在任何宿主调用前拒绝', () => {
  for (const step of [sequence, {kind:'tap', key:65}, {kind:'delay', milliseconds:0}]) {
    const good = fixture({allowedProcesses:['code'], steps:Array(31).fill(step), keyDownDelayMs:0});
    const result = main(good.ctx);
    assert.equal(result.steps, 31); assert.equal(good.calls.length, 32);
    assert.equal(result.inputSubmitted, step.kind !== 'delay');
    const bad = fixture({allowedProcesses:['code'], steps:Array(32).fill(step), keyDownDelayMs:0});
    assert.throws(() => main(bad.ctx), invalid); assert.deepEqual(bad.calls, []);
  }
});

test('后续错误步骤在目标查询和前面输入前拒绝', () => {
  const f = fixture({allowedProcesses:['code'], steps:[{kind:'text', text:'不得先发送'}, {kind:'tap', key:0}]});
  assert.throws(() => main(f.ctx), invalid); assert.deepEqual(f.calls, []);
});

test('目标查询期间改写名单、步骤和时序不会改变快照', () => {
  const f = fixture({allowedProcesses:['code'], steps:[sequence, {kind:'text', text:'原文'}]}, {onTarget(ctx) {
    ctx.input.allowedProcesses[0] = 'other'; ctx.input.steps[0].keys[0] = 87;
    ctx.input.steps[0].modifiers[0] = 163; ctx.input.steps[1].text = '替换';
    ctx.input.steps.push({kind:'tap', key:13}); ctx.input.keyDownDelayMs = 999;
  }});
  assert.deepEqual(main(f.ctx), {inputSubmitted:true, steps:2});
  assert.deepEqual(f.calls, [['target'], ['sequence', [162], [75, 67], {keyDownDelayMs:0}], ['text', '原文']]);
  assert.equal(Object.isFrozen(f.ctx.input.allowedProcesses), false);
});

test('目标错误和派发错误原样传播，不重新查询或重发', () => {
  const fault = Object.assign(new Error('原目标变化'), {code:'target_changed', effectState:'not_started'});
  for (const where of ['targetError', 'inputError']) {
    const f = fixture({allowedProcesses:['code'], steps:[sequence, {kind:'text', text:'不得继续'}]}, {[where]:fault});
    assert.throws(() => main(f.ctx), error => error === fault);
    assert.equal(f.calls.length, where === 'targetError' ? 1 : 2);
  }
});
