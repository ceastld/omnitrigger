// RecordingHost不创建子进程，不调用真实宿主或通知。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
import asyncMain from './main.mjs';
import {main as syncMain} from './main-script.mjs';

const program='C:\\OwnedFixture\\program.exe';
const cwd='C:\\OwnedFixture';
const output='  合成标准输出 文😀\n ';
async function run(capture,input={program,cwd},{result={code:0,stdout:output,stderr:'synthetic err'},receipt={submitted:true},failAt,error}={}){
  const calls=[];
  const invoke=(method,args,value)=>{
    calls.push([method,args]);
    const perform=()=>{if(method===failAt)throw error;return value;};
    return capture?perform():Promise.resolve().then(perform);
  };
  const ctx={input,process:{run:(...args)=>invoke('process.run',args,result),
    capture:(...args)=>invoke('process.capture',args,result)},ui:{notify:params=>invoke('ui.notify',[params],receipt)}};
  try{return{output:capture?syncMain(ctx):await asyncMain(ctx),calls};}
  catch(error){return{error,calls};}
}

for(const capture of [false,true]){
  const name=capture?'轻量capture':'Node run';
  test(`${name} 只运行一次，随后显示stdout，终态只摘要`,async()=>{
    const result=await run(capture);
    assert.deepEqual(result.calls.map(row=>row[0]),[capture?'process.capture':'process.run','ui.notify']);
    assert.deepEqual(result.calls[0][1],[program,[],{cwd,allowNonZero:false,...(capture?{timeoutMs:10000}:{})}]);
    assert.equal(result.calls[1][1][0].message,output.trim());
    assert.deepEqual(result.output,{completed:true,exitCode:0,stdoutUtf16Units:output.length,stderrUtf16Units:13,notified:true});
    for(const secret of [program,cwd,output,'synthetic err'])assert.equal(JSON.stringify(result.output).includes(secret),false);
  });

  test(`${name} 参数错误在运行前拒绝，工作目录必须明确`,async()=>{
    for(const input of [{program},{program:'python',cwd},{program:'C:\\run.cmd',cwd},{program,cwd:'relative'},
      {program,cwd,args:[1]},{program,cwd,args:['\0']},{program,cwd,trimOutput:1},
      {program,cwd,allowNonZero:'true'},{program,cwd,extra:1}]){
      const result=await run(capture,input);
      assert.equal(result.error?.code,'invalid_input');assert.deepEqual(result.calls,[]);
    }
    const timeout=await run(capture,{program,cwd,timeoutMs:100});
    if(capture)assert.equal(timeout.output.notified,true);
    else{assert.equal(timeout.error.code,'invalid_input');assert.deepEqual(timeout.calls,[]);}
  });

  test(`${name} 非零策略明确，不替换程序或重试`,async()=>{
    const result={code:7,stdout:'controlled warning',stderr:'stderr'};
    const rejected=await run(capture,{program,cwd},{result});
    assert.equal(rejected.error.code,'process_exit_nonzero');assert.equal(rejected.calls.length,1);
    const allowed=await run(capture,{program,cwd,allowNonZero:true},{result});
    assert.equal(allowed.calls[1][1][0].level,'warning');assert.equal(allowed.output.exitCode,7);
  });

  test(`${name} 空输出固定提示，过大/控制字符输出不截断或泄漏`,async()=>{
    const empty=await run(capture,{program,cwd},{result:{code:0,stdout:' \r\n ',stderr:''}});
    assert.equal(empty.calls[1][1][0].message,'程序已完成，没有标准输出。');
    for(const stdout of ['x'.repeat(1025),'secret\0value','secret\x01value']){
      const result=await run(capture,{program,cwd},{result:{code:0,stdout,stderr:''}});
      assert.equal(result.error.code,'output_not_displayable');assert.equal(result.error.effectState,'unknown');
      assert.equal(result.error.message.includes('secret'),false);assert.equal(result.calls.length,1);
    }
    const preserve=await run(capture,{program,cwd,trimOutput:false});
    assert.equal(preserve.calls[1][1][0].message,output);
  });

  test(`${name} 运行取消和后续通知失败保留效果，无二次执行`,async()=>{
    const fault=Object.assign(new Error(output),{code:'cancelled',effectState:'not_started'});
    const process=await run(capture,{program,cwd},{failAt:capture?'process.capture':'process.run',error:fault});
    assert.equal(process.error.code,'cancelled');assert.equal(process.calls.length,1);
    const notify=await run(capture,{program,cwd},{failAt:'ui.notify',error:fault});
    assert.equal(notify.error.effectState,'unknown');assert.equal(notify.calls.length,2);
    assert.equal(notify.error.message.includes(output),false);
    const malformed=await run(capture,{program,cwd},{receipt:{}});
    assert.equal(malformed.error.code,'invalid_response');assert.equal(malformed.error.effectState,'unknown');
  });
}

test('旧轻量宿主缺capture时明确失败，不偷偷回退run',()=>{
  let called=false;
  const ctx={input:{program,cwd},process:{run:()=>{called=true;}},ui:{notify:()=>{called=true;}}};
  assert.throws(()=>syncMain(ctx),error=>error.code==='process_capture_unavailable'&&error.effectState==='not_started');
  assert.equal(called,false);
});

test('新SDK的capture和通知桥接可组合，仍只使用合成JSON响应',()=>{
  const runtime=new URL('../../../crates/omnitrigger-actions/runtime/',import.meta.url);
  const source=['desktop-api.js','browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js','state-api.js'].map(name=>readFileSync(new URL(name,runtime),'utf8')).join('\n')
    +'\n'+readFileSync(new URL('../../../tools/js-engine-probe/src/sdk.js',import.meta.url),'utf8');
  const calls=[];
  const scope={ctx:{input:{program,cwd},__nativeContext:{connection:{port:1234,token:'a'.repeat(64)}}},__hostCall:text=>{
    const value=JSON.parse(text);calls.push(value);
    return JSON.stringify({ok:true,value:value.kind==='capture'?{code:0,stdout:output,stderr:''}:{submitted:true}});
  }};
  runInNewContext(source,scope);
  assert.equal(syncMain(scope.ctx).notified,true);
  assert.deepEqual(calls.map(value=>value.kind==='capture'?'process.capture':value.method),['process.capture','ui.notify']);
});

test('两份调用明确区分timeout策略，清单入口独立',()=>{
  const manifest=JSON.parse(readFileSync(new URL('action.json',import.meta.url),'utf8'));
  for(const name of ['call.json','call-script.json']){
    const value=JSON.parse(readFileSync(new URL(name,import.meta.url),'utf8'));
    const action=manifest.actions.find(action=>action.id===value.action_id);
    assert.equal(action.single_instance,true);assert.equal(action.version,value.action_version);
    assert.equal(Object.hasOwn(value.input,'timeoutMs'),action.runtime==='script');
  }
});
