// 仅合成Promise与RecordingHost，不运行真实程序或显示通知。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import main from './main-capture.mjs';
import oldNode from './main.mjs';
import {main as oldScript} from './main-script.mjs';
const input={program:'C:\\OwnedFixture\\capture-fixture.exe',args:['exit','0'],cwd:'C:\\OwnedFixture',timeoutMs:2500};
const value={code:0,stdout:'  合成输出 😀\n ',stderr:'合成stderr'};
const deferred=()=>{let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return{promise,resolve,reject};};
const failure=(code,effectState='unknown')=>Object.assign(new Error('私有合成内容'),{code,effectState});

test('Node入口按顺序await capture和通知，传capture选项，只返回摘要',async()=>{
  const capture=deferred(),notify=deferred(),calls=[];let settled=false;
  const ctx={input,process:{capture:(...args)=>{calls.push(['capture',args]);return capture.promise;},run(){throw new Error('不允许run回退');}},
    ui:{notify:params=>{calls.push(['notify',params]);return notify.promise;}}};
  const result=main(ctx);result.then(()=>{settled=true;});
  assert.deepEqual(calls.map(call=>call[0]),['capture']);assert.equal(settled,false);
  assert.deepEqual(calls[0][1],[input.program,input.args,{cwd:input.cwd,allowNonZero:false,timeoutMs:2500}]);
  capture.resolve(value);await Promise.resolve();await Promise.resolve();
  assert.deepEqual(calls.map(call=>call[0]),['capture','notify']);assert.equal(settled,false);
  assert.equal(calls[1][1].message,value.stdout.trim());notify.resolve({submitted:true});
  assert.deepEqual(await result,{completed:true,exitCode:0,stdoutUtf16Units:value.stdout.length,stderrUtf16Units:value.stderr.length,notified:true});
});
test('capture缺失或参数非法先拒绝，不调用run或通知',async()=>{
  for(const ctx of [
    {input,process:{run(){throw new Error('禁止回退');}},ui:{notify(){throw new Error('不应通知');}}},
    {input:{...input,cwd:'relative'},process:{capture(){throw new Error('不应启动');}},ui:{notify(){throw new Error('不应通知');}}},
  ])await assert.rejects(main(ctx),error=>['process_capture_unavailable','invalid_input'].includes(error.code)&&error.effectState==='not_started');
});
test('采集拒绝不通知，通知拒绝保留程序效果，均无重试或正文泄漏',async()=>{
  for(const failAt of ['capture','notify']){
    const calls=[];const fault=failure(failAt==='capture'?'process_timeout':'ui_error',failAt==='capture'?'unknown':'not_started');
    const ctx={input,process:{capture:async()=>{calls.push('capture');if(failAt==='capture')throw fault;return value;}},
      ui:{notify:async()=>{calls.push('notify');throw fault;}}};
    await assert.rejects(main(ctx),error=>error.code===fault.code&&error.effectState==='unknown'
      &&error.method===(failAt==='capture'?'process.capture':'ui.notify')&&!error.message.includes('私有'));
    assert.deepEqual(calls,failAt==='capture'?['capture']:['capture','notify']);
  }
});
test('新增动作/调用明确Node capture；旧notify和notify-script行为保留',async()=>{
  const manifest=JSON.parse(readFileSync(new URL('action.json',import.meta.url),'utf8'));
  const call=JSON.parse(readFileSync(new URL('call-capture.json',import.meta.url),'utf8'));
  assert.equal(manifest.actions.length,3);assert.equal(call.action_id,'notify-capture');
  assert.deepEqual(manifest.actions.map(action=>[action.id,action.runtime,action.entry,action.version]),[
    ['notify','node','main.mjs',1],['notify-script','script','main-script.mjs',1],['notify-capture','node','main-capture.mjs',1],
  ]);
  const action=manifest.actions.find(action=>action.id===call.action_id);
  assert.equal(action.single_instance,true);assert.equal(call.package_id,manifest.id);assert.equal(call.action_version,action.version);
  assert.equal(call.input.timeoutMs,10000);
  const legacyInput={program:input.program,args:input.args,cwd:input.cwd};const calls=[];
  await oldNode({input:legacyInput,process:{run:async(...args)=>{calls.push(['run',args]);return value;}},ui:{notify:async()=>({submitted:true})}});
  oldScript({input:{...legacyInput,timeoutMs:2500},process:{capture:(...args)=>{calls.push(['script-capture',args]);return value;}},ui:{notify:()=>({submitted:true})}});
  assert.deepEqual(calls.map(call=>call[0]),['run','script-capture']);
  assert.equal(Object.hasOwn(calls[0][1][2],'timeoutMs'),false);assert.equal(calls[1][1][2].timeoutMs,2500);
  await assert.rejects(oldNode({input,process:{},ui:{}}),error=>error.code==='invalid_input');
});
