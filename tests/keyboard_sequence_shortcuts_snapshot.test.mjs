import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import sequence from '../examples/actions/keyboard-sequence/sequence.mjs';
import shortcuts from '../examples/actions/keyboard-sequence/shortcuts.mjs';

const sdkSource = await readFile(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js', import.meta.url), 'utf8');
const create = new Function(`${sdkSource}; return createDesktopApi;`)();
const keys = { modifiers:[163,161], keys:[84] };
const steps = [{keys:[162,84],delayAfterMs:10},{keys:[162,87],delayAfterMs:0}];
function fixture(input, {onTarget,onShortcut,failAt} = {}) {
  const calls = []; let sent = 0;
  const ctx = { input:structuredClone(input), desktop:null, async delay(ms) {
    if (!Number.isInteger(ms) || ms < 0 || ms > 60000) throw Object.assign(new Error('等待'), {code:'invalid_input'});
    calls.push(['delay',ms]);
  } };
  const api = create(async (method, params) => {
    if (method === 'target.info') { calls.push([method]); onTarget?.(ctx); return {processName:'WPS.exe'}; }
    calls.push([method,structuredClone(params)]);
    if (method === 'keyboard.shortcut') onShortcut?.(ctx,++sent);
    if (failAt === method) throw Object.assign(new Error('目标已变化'), {code:'target_changed',effectState:'not_started'});
    return {submitted:true};
  });
  // 与真实 Node facade 一样：公开 SDK 的同步参数错误转为 Promise rejection。
  ctx.desktop = Object.fromEntries(Object.entries(api.desktop).map(([name,fn]) => [name,async (...args) => fn(...args)]));
  return {ctx,calls};
}

test('sequence 默认目标等待期间修改原数组，不改变最初左右键', async () => {
  const f=fixture(keys,{onTarget:ctx=>{ctx.input.modifiers[0]=162;ctx.input.keys[0]=87;}});
  assert.deepEqual(await sequence(f.ctx),{inputSubmitted:true});
  assert.deepEqual(f.calls,[['target.info'],['keyboard.sequence',{modifiers:[163,161],keys:[84],key_down_delay_ms:10}]]);
});

test('shortcuts 在目标等待前固定嵌套keys、步骤数和每步等待', async () => {
  const f=fixture({steps},{onTarget:ctx=>{ctx.input.steps[0].keys[1]=87;ctx.input.steps[0].delayAfterMs=500;ctx.input.steps.push({keys:[13]});}});
  assert.deepEqual(await shortcuts(f.ctx),{inputSubmitted:true,steps:2});
  assert.deepEqual(f.calls,[['target.info'],['keyboard.shortcut',{keys:[162,84],key_down_delay_ms:10}],['delay',10],['keyboard.shortcut',{keys:[162,87],key_down_delay_ms:10}],['delay',0]]);
});

test('首步派发期间修改其等待和后续keys，不改变已固定计划', async () => {
  const f=fixture({steps,keyDownDelayMs:0},{onShortcut:(ctx,n)=>{if(n===1){ctx.input.steps[0].delayAfterMs=500;ctx.input.steps[1].keys[1]=13;ctx.input.keyDownDelayMs=1000;}}});
  await shortcuts(f.ctx);
  assert.deepEqual(f.calls,[['keyboard.shortcut',{keys:[162,84]}],['delay',10],['keyboard.shortcut',{keys:[162,87]}],['delay',0]]);
});

for(const [name,step] of Object.entries({empty:{keys:[]},range:{keys:[255]},fraction:{keys:[65.5]},tooMany:{keys:[1,2,3,4,5,6]},negativeDelay:{keys:[65],delayAfterMs:-1},highDelay:{keys:[65],delayAfterMs:60001},nullDelay:{keys:[65],delayAfterMs:null}})) {
  test(`shortcuts 后续 ${name} 错误在任何原目标查询或派发前拒绝`,async()=>{
    const f=fixture({steps:[{keys:[162,84]},step]});
    await assert.rejects(shortcuts(f.ctx),e=>e.code==='invalid_input'&&e.effectState==='not_started');
    assert.deepEqual(f.calls,[]);
  });
}

test('sequence 形状及显式等待预算先拒绝，不先查询原目标',async()=>{
  for(const input of [{modifiers:[],keys:[]},{...keys,keyDownDelayMs:-1},{modifiers:[],keys:[65,66,67],keyDownDelayMs:1000}]){
    const f=fixture(input);await assert.rejects(sequence(f.ctx),e=>e.code==='invalid_input');assert.deepEqual(f.calls,[]);
  }
});

test('两入口保留显式0、顺序和原API参数，不查询目标或新增timing业务字段',async()=>{
  const a=fixture({...keys,keyDownDelayMs:0});await sequence(a.ctx);
  assert.deepEqual(a.calls,[['keyboard.sequence',{modifiers:[163,161],keys:[84],key_down_delay_ms:0}]]);
  const b=fixture({steps:[{keys:[163,84]}],keyDownDelayMs:0});await shortcuts(b.ctx);
  assert.deepEqual(b.calls,[['keyboard.shortcut',{keys:[163,84]}]]);
});

test('宿主目标变化时停止剩余步骤，不重试',async()=>{
  const f=fixture({steps,keyDownDelayMs:0},{failAt:'keyboard.shortcut'});
  await assert.rejects(shortcuts(f.ctx),e=>e.code==='target_changed');assert.equal(f.calls.length,1);
});

test('入口不改写或冻结调用方对象',async()=>{
  for(const [main,input] of [[sequence,{...keys,keyDownDelayMs:0}],[shortcuts,{steps,keyDownDelayMs:0}]]){
    const f=fixture(input);const before=structuredClone(f.ctx.input);await main(f.ctx);
    assert.deepEqual(f.ctx.input,before);assert.equal(Object.isFrozen(f.ctx.input),false);
  }
});

test('shortcut数值/数量/时序边界继续由当前公开SDK判定，不新增键位语义',async()=>{
  for(const [keyList,delay] of [[[1],0],[[254],1000],[[162,162,65],0],[[],0],[[255],0],[[1,2,3,4,5,6],0],[[65],1001],[[65],null]]){
    const api=create(()=>({submitted:true}));let expected;
    try{api.desktop.sendShortcut(keyList,{keyDownDelayMs:delay});expected='ok';}catch(e){expected=e.code;}
    const f=fixture({steps:[{keys:keyList}],keyDownDelayMs:delay});
    if(expected==='ok')await shortcuts(f.ctx);else await assert.rejects(shortcuts(f.ctx),e=>e.code===expected);
  }
});

test('sequence数量与joint等待边界与公开SDK一致，不自行规定修饰键组合',async()=>{
  for(const input of [{modifiers:[],keys:[1],keyDownDelayMs:0},{modifiers:[254],keys:[65],keyDownDelayMs:1000},{modifiers:[1,2,3,4,5],keys:[65],keyDownDelayMs:0},{modifiers:[],keys:[65,66],keyDownDelayMs:1000},{modifiers:[],keys:[65,66,67],keyDownDelayMs:1000}]){
    const api=create(()=>({submitted:true}));let expected;
    try{api.desktop.sendKeySequence(input.modifiers,input.keys,{keyDownDelayMs:input.keyDownDelayMs});expected='ok';}catch(e){expected=e.code;}
    const f=fixture(input);if(expected==='ok')await sequence(f.ctx);else await assert.rejects(sequence(f.ctx),e=>e.code===expected);
  }
});
