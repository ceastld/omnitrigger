import foldNode from '../examples/actions/keyboard-sequence/fold-all.mjs';
import {main as foldScript} from '../examples/actions/keyboard-sequence/fold-all-script.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
import recipe from '../examples/actions/keyboard-sequence/recipe.mjs';

const first = { kind: 'text', text: '不可提前发送' };
const sequence = { kind: 'sequence', modifiers: [163, 161], keys: [84] };
function fixture(input, { onTarget, failAt } = {}) {
  const calls = [];
  const ctx = { input: structuredClone(input), desktop: {
    async target() { calls.push(['target']); onTarget?.(ctx); return { processName: 'WPS.exe' }; },
    async sendKeySequence(modifiers, keys, options) {
      if (!Array.isArray(modifiers) || !Array.isArray(keys) || !keys.length || [...modifiers, ...keys].some(v => !Number.isInteger(v) || v < 1 || v > 254)) {
        throw Object.assign(new Error('无效按键'), { code: 'invalid_input' });
      }
      calls.push(structuredClone(['sequence', modifiers, keys, options]));
      if (failAt === 'sequence') throw Object.assign(new Error('目标变化'), { code: 'target_changed', effectState: 'not_started' });
      return { submitted: true };
    },
    async sendText(text) {
      if (typeof text !== 'string' || text.includes('\0') || /[\ud800-\udfff]/u.test(text.replace(/[\ud800-\udbff][\udc00-\udfff]/gu, ''))) {
        throw Object.assign(new Error('无效文本'), { code: 'invalid_input' });
      }
      calls.push(['text', text]); return { submitted: true };
    },
  }, async delay(milliseconds) {
    if (!Number.isInteger(milliseconds) || milliseconds < 0 || milliseconds > 60000) throw Object.assign(new Error('无效等待'), { code: 'invalid_input' });
    calls.push(['delay', milliseconds]);
  } };
  return { ctx, calls };
}

for (const [name, step] of Object.entries({
  emptyKeys: { ...sequence, keys: [] }, noModifiers: { kind: 'sequence', keys: [84] },
  badVK: { ...sequence, keys: [0] }, tooMany: { ...sequence, keys: Array(17).fill(84) },
  nullTap: { kind: 'tap', key: null }, fractionalTap: { kind: 'tap', key: 65.1 },
  negativeWait: { kind: 'delay', milliseconds: -1 }, excessiveWait: { kind: 'delay', milliseconds: 60001 },
  nulText: { kind: 'text', text: 'abc\0' }, surrogate: { kind: 'text', text: '\ud800' },
  excessiveText: { kind: 'text', text: 'x'.repeat(65537) },
})) test(`后续 ${name} 错误在任何输入或目标查询前拒绝`, async () => {
  const f = fixture({ steps: [first, step] });
  await assert.rejects(recipe(f.ctx), e => e.code === 'invalid_input' && e.effectState === 'not_started');
  assert.deepEqual(f.calls, []);
});

for (const value of [-1, 1001, 1.2, null, '10']) test(`无效全局按下延迟 ${value} 在文本执行前拒绝`, async () => {
  const f = fixture({ steps: [first], keyDownDelayMs: value });
  await assert.rejects(recipe(f.ctx), e => e.code === 'invalid_input');
  assert.deepEqual(f.calls, []);
});

test('单次等待预算超限先拒绝，不先发送前面的文本', async () => {
  const f = fixture({ steps: [first, { ...sequence, keys: [65, 66, 67] }], keyDownDelayMs: 1000 });
  await assert.rejects(recipe(f.ctx), e => e.code === 'invalid_input');
  assert.deepEqual(f.calls, []);
});

test('默认目标查询计入 32 次预算，超限前不派发', async () => {
  const f = fixture({ steps: Array.from({ length: 32 }, () => structuredClone(sequence)) });
  await assert.rejects(recipe(f.ctx), e => e.code === 'invalid_input');
  assert.deepEqual(f.calls, []);
});

test('显式 0 保留，32 个有序步骤无需目标查询', async () => {
  const f = fixture({ steps: Array.from({ length: 32 }, () => structuredClone(sequence)), keyDownDelayMs: 0 });
  assert.deepEqual(await recipe(f.ctx), { inputSubmitted: true, steps: 32 });
  assert.equal(f.calls.length, 32); assert.ok(f.calls.every(c => c[0] === 'sequence' && c[3].keyDownDelayMs === 0));
});

test('恢复网页原左右修饰键和步骤顺序保持，WPS 默认 10ms', async () => {
  const f = fixture({ steps: [sequence, { kind: 'delay', milliseconds: 30 }, { kind: 'tap', key: 8 }, { kind: 'text', text: '中文😀' }] });
  assert.deepEqual(await recipe(f.ctx), { inputSubmitted: true, steps: 4 });
  assert.deepEqual(f.calls, [['target'], ['sequence', [163, 161], [84], { keyDownDelayMs: 10 }], ['delay', 30], ['sequence', [], [8], { keyDownDelayMs: 0 }], ['text', '中文😀']]);
});

test('等待默认目标时快照步骤与数组，后续改写不会改变按键', async () => {
  const f = fixture({ steps: [sequence, { kind: 'text', text: '原文' }] }, { onTarget: ctx => {
    ctx.input.steps[0].keys[0] = 87; ctx.input.steps[0].modifiers[0] = 162;
    ctx.input.steps[1].text = '替换'; ctx.input.steps.push({ kind: 'tap', key: 13 }); ctx.input.keyDownDelayMs = 999;
  } });
  assert.deepEqual(await recipe(f.ctx), { inputSubmitted: true, steps: 2 });
  assert.deepEqual(f.calls, [['target'], ['sequence', [163, 161], [84], { keyDownDelayMs: 10 }], ['text', '原文']]);
});

test('只等待的配方不会谎报已提交输入，不查询目标', async () => {
  const f = fixture({ steps: [{ kind: 'delay', milliseconds: 0 }, { kind: 'delay', milliseconds: 10 }] });
  assert.deepEqual(await recipe(f.ctx), { inputSubmitted: false, steps: 2 });
  assert.deepEqual(f.calls, [['delay', 0], ['delay', 10]]);
});

test('宿主目标变化后停止，不重试或继续文本', async () => {
  const f = fixture({ steps: [sequence, first], keyDownDelayMs: 0 }, { failAt: 'sequence' });
  await assert.rejects(recipe(f.ctx), e => e.code === 'target_changed');
  assert.equal(f.calls.length, 1);
});

test('普通合法参数不会改写或冻结调用方步骤', async () => {
  const f = fixture({ steps: [sequence, { kind: 'text', text: '' }], keyDownDelayMs: 0 });
  const before = structuredClone(f.ctx.input); await recipe(f.ctx);
  assert.deepEqual(f.ctx.input, before); assert.equal(Object.isFrozen(f.ctx.input.steps), false);
});

test('与既有同步配方核对普通有效/无效参数及输出', async () => {
  const source = await readFile(new URL('../examples/actions/keyboard-sequence/recipe.js', import.meta.url), 'utf8');
  const main = runInNewContext(`${source}\nmain;`);
  for (const input of [
    { steps: [sequence], keyDownDelayMs: 0 }, { steps: [{ kind: 'delay', milliseconds: 0 }] },
    { steps: [first, { kind: 'tap', key: 0 }] }, { steps: [first, { kind: 'text', text: '\udfff' }] },
    { steps: [first], keyDownDelayMs: -1 },
  ]) {
    const calls = [];
    const syncCtx = { input: structuredClone(input), desktop: {
      target: () => ({ processName: 'WPS.exe' }),
      sendKeySequence: (...args) => calls.push(['sequence', ...args]), sendText: text => calls.push(['text', text]),
    }, delay: ms => calls.push(['delay', ms]) };
    const f = fixture(input);
    let syncResult; let syncError;
    try { syncResult = main(syncCtx); } catch (e) { syncError = e; }
    if (syncError) { await assert.rejects(recipe(f.ctx), e => e.code === syncError.code); assert.deepEqual(f.calls, []); }
    else { assert.deepEqual(await recipe(f.ctx), structuredClone(syncResult)); assert.deepEqual(f.calls, structuredClone(calls)); }
  }
});

for(const [runtime,run] of [['fold-all Node',foldNode],['fold-all Script',foldScript]]) {
    function fixture({input={},failAt,missingAckAt}={}) {
        const calls=[];let count=0;
        const ctx={input,desktop:{sendShortcut(keys,options){
            count++;calls.push({kind:'shortcut',keys:Array.from(keys),options:{...options}});
            if(failAt===count)throw {code:'target_changed',method:'keyboard.shortcut',effectState:'not_started'};
            return missingAckAt===count?{}:{submitted:true};
        }},delay(milliseconds){calls.push({kind:'delay',milliseconds});if(failAt==='delay')throw {code:'cancelled',method:'delay',effectState:'not_started'};}};
        return {ctx,calls};
    }
    test(runtime+'：固定序列、独立修饰键和50ms延迟完整保留',async()=>{
        const f=fixture();const result=await run(f.ctx);
        assert.deepEqual(f.calls,[{kind:'shortcut',keys:[162,77],options:{keyDownDelayMs:0}},{kind:'delay',milliseconds:50},{kind:'shortcut',keys:[162,76],options:{keyDownDelayMs:0}}]);
        assert.equal(result.acceptedSubmissions,2);assert.equal(result.inputSubmitted,true);
    });
    test(runtime+'：未知参数在任何输入前拒绝',async()=>{
        for(const input of [null,[],{keys:[162,77]},{delay:0}]){
            const f=fixture({input});await assert.rejects(async()=>run(f.ctx),e=>e.code==='invalid_input'&&e.effectState==='not_started');assert.deepEqual(f.calls,[]);
        }
    });
    test(runtime+'：首段目标失败不等待或重试',async()=>{
        const f=fixture({failAt:1});await assert.rejects(async()=>run(f.ctx),e=>e.code==='target_changed'&&e.acceptedSubmissions===0&&e.effectState==='not_started');assert.equal(f.calls.length,1);
    });
    test(runtime+'：首段确认未知不派发后续',async()=>{
        const f=fixture({missingAckAt:1});await assert.rejects(async()=>run(f.ctx),e=>e.code==='submission_unknown'&&e.effectState==='unknown');assert.equal(f.calls.length,1);
    });
    test(runtime+'：等待时取消保留首段部分提交语义',async()=>{
        const f=fixture({failAt:'delay'});await assert.rejects(async()=>run(f.ctx),e=>e.code==='cancelled'&&e.effectState==='unknown'&&e.acceptedSubmissions===1&&e.phase==='inter_chord_delay'&&e.message.includes('已确认1段'));assert.equal(f.calls.length,2);
    });
    test(runtime+'：末段目标变化不重发首段或末段',async()=>{
        const f=fixture({failAt:2});await assert.rejects(async()=>run(f.ctx),e=>e.code==='target_changed'&&e.effectState==='unknown'&&e.stepEffectState==='not_started'&&e.acceptedSubmissions===1&&e.message.includes('末段按键未确认'));assert.equal(f.calls.length,3);
    });
    test(runtime+'：末段确认未知不补偿',async()=>{
        const f=fixture({missingAckAt:2});await assert.rejects(async()=>run(f.ctx),e=>e.code==='submission_unknown'&&e.effectState==='unknown'&&e.acceptedSubmissions===1);assert.equal(f.calls.length,3);
    });
}
