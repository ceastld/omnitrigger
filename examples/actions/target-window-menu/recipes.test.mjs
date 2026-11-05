import assert from 'node:assert/strict';
import test from 'node:test';
import asyncMain from './main.mjs';
import {main as syncMain} from './main-script.mjs';

async function run(asynchronous, {input = {}, choice = {status:'selected',id:'minimize'}, receipt = {submitted:true}, failAt, error} = {}) {
  const calls = [];
  const invoke = (method, args, value) => {
    calls.push([method,args]);
    const result = () => {if (method === failAt) throw error; return value;};
    return asynchronous ? Promise.resolve().then(result) : result();
  };
  const ctx = {input,ui:{choose: params => invoke('ui.choose',[params],choice)},
    desktop:{windowAction: operation => invoke('window.action',[operation],receipt)}};
  try { return {output:asynchronous ? await asyncMain(ctx) : syncMain(ctx),calls}; }
  catch (error) { return {error,calls}; }
}

for (const asynchronous of [false,true]) {
  const name = asynchronous ? 'Node' : '同步';
  test(`${name} 三种窗口操作只在选择后提交一次`, async () => {
    for (const id of ['minimize','toggle_maximize','close']) {
      const result = await run(asynchronous,{choice:{status:'selected',id}});
      assert.deepEqual(result.output,{status:'submitted',operation:id,submitted:true});
      assert.deepEqual(result.calls.map(call => call[0]),['ui.choose','window.action']);
      assert.deepEqual(result.calls[1][1],[id]);
      assert.deepEqual(result.calls[0][1][0].items.map(item => item.id),['minimize','toggle_maximize','close']);
    }
  });

  test(`${name} 关闭或超时不提交操作`, async () => {
    for (const reason of ['dismissed','timeout']) {
      const result = await run(asynchronous,{choice:{status:'cancelled',reason}});
      assert.deepEqual(result.output,{status:'cancelled',submitted:false});
      assert.equal(result.calls.length,1);
    }
  });

  test(`${name} 非空参数和未知选择不能控制目标或改变业务`, async () => {
    for (const input of [null,[],{operation:'close'},{windowId:'123'},{text:'secret'}]) {
      const result = await run(asynchronous,{input});
      assert.equal(result.error.code,'invalid_input');
      assert.equal(result.calls.length,0);
    }
    for (const choice of [null,{status:'selected',id:'terminate_process'},
      {status:'selected',id:'close',windowId:'123'},{status:'cancelled',reason:'other'}]) {
      const result = await run(asynchronous,{choice});
      assert.equal(result.error.code,'invalid_response');
      assert.equal(result.calls.length,1);
    }
  });

  test(`${name} 失焦、取消和已提交后的畸形结果均无重试`, async () => {
    for (const code of ['target_changed','cancelled','timeout']) {
      const error = Object.assign(new Error('synthetic private title'),{code,effectState:'not_started'});
      const result = await run(asynchronous,{failAt:'window.action',error});
      assert.equal(result.error.code,code);
      assert.equal(result.error.message.includes('private'),false);
      assert.equal(result.calls.length,2);
    }
    const result = await run(asynchronous,{receipt:{}});
    assert.equal(result.error.code,'invalid_response');
    assert.equal(result.error.effectState,'unknown');
    assert.equal(result.calls.length,2);
  });
}
