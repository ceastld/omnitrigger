// 双入口只接入合成响应；VM 中的轻量 SDK 不创建原生宿主或子进程。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import {runInNewContext} from 'node:vm';
import nodeMain from './main.mjs';
import {main as scriptMain} from './main-script.mjs';

const runtime=new URL('../../../crates/omnitrigger-actions/runtime/',import.meta.url);
const createDesktopApi=runInNewContext(readFileSync(new URL('desktop-api.js',runtime),'utf8')+'\ncreateDesktopApi;');
const sdk=['desktop-api.js','browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js']
  .map(name=>readFileSync(new URL(name,runtime),'utf8')).join('\n')
  +'\n'+readFileSync(new URL('../../../tools/js-engine-probe/src/sdk.js',import.meta.url),'utf8');
const input={program:'C:\\OwnedFixture\\program.exe',cwd:'C:\\OwnedFixture',args:['','空 格','"quoted"','C:\\tail\\']};
const plain=value=>JSON.parse(JSON.stringify(value));

async function invoke(script,parameters=input,{result={code:0,stdout:'  合成输出😀\r\n',stderr:'仅计数'},failure,notifyFailure}={}){
  const calls=[];
  const notify=params=>{
    calls.push({method:'ui.notify',params:plain(params)});
    if(notifyFailure)throw Object.assign(new Error('私有通知错误'),notifyFailure);
    return {submitted:true};
  };
  let ctx;
  if(script){
    const scope={ctx:{input:structuredClone(parameters),__nativeContext:{connection:{port:1234,token:'a'.repeat(64)}}},__hostCall:request=>{
      const value=JSON.parse(request);
      if(value.kind==='capture'){
        calls.push({method:'process.capture',params:value});
        return JSON.stringify(failure?{ok:false,error:{...failure,effect_state:failure.effectState,message:'私有程序错误'}}:{ok:true,value:result});
      }
      assert.equal(value.kind,'native');
      assert.equal(value.method,'ui.notify');
      // UI 回执也通过轻量 SDK 的既有 JSON 桥接。
      try{return JSON.stringify({ok:true,value:notify(value.params)});}
      catch(error){return JSON.stringify({ok:false,error:{code:error.code,message:error.message,effect_state:error.effectState}});}
    }};
    runInNewContext(sdk,scope);ctx=scope.ctx;
  }else{
    ctx={input:structuredClone(parameters),process:{run:async(program,args,options)=>{
      calls.push({method:'process.run',params:{program,args,options}});
      if(failure)throw Object.assign(new Error('私有程序错误'),failure);
      return result;
    }},ui:{notify:async params=>createDesktopApi((method,value)=>{
      assert.equal(method,'ui.notify');return notify(value);
    }).ui.notify(params)}};
  }
  try{return{value:plain(script?scriptMain(ctx):await nodeMain(ctx)),calls};}
  catch(error){return{error:{code:error.code,method:error.method,effectState:error.effectState,message:error.message},calls};}
}

test('双入口经各自适配传递相同显式目录与参数，产生相同通知和摘要',async()=>{
  const node=await invoke(false);const script=await invoke(true);
  assert.deepEqual(script.value,node.value);
  assert.deepEqual(script.calls.slice(1),node.calls.slice(1));
  const wire=script.calls[0].params;const nodeParams=node.calls[0].params;
  assert.deepEqual([wire.program,wire.args,wire.cwd],[nodeParams.program,nodeParams.args,nodeParams.options.cwd]);
  assert.equal(wire.timeout_ms,10000);assert.equal(wire.allow_non_zero,false);
  assert.equal(Object.hasOwn(nodeParams.options,'timeoutMs'),false);
  assert.equal(node.calls.length,2);assert.equal(script.calls.length,2);
});

test('两入口非零策略一致，显式允许后才通知，不依赖 Node 假宿主代为拒绝',async()=>{
  const result={code:7,stdout:'显式警告',stderr:'不显示'};
  for(const script of [false,true]){
    const rejected=await invoke(script,input,{result});
    assert.equal(rejected.error.code,'process_exit_nonzero');assert.equal(rejected.error.effectState,'unknown');
    assert.equal(rejected.calls.length,1);
    const allowed=await invoke(script,{...input,allowNonZero:true},{result});
    assert.equal(allowed.value.exitCode,7);assert.equal(allowed.calls[1].params.level,'warning');
  }
});

test('两入口派发或通知失败只保留结构化结果，不重试或泄漏响应正文',async()=>{
  for(const script of [false,true]){
    const failed=await invoke(script,input,{failure:{code:'process_timeout',effectState:'unknown'}});
    assert.equal(failed.error.code,'process_timeout');assert.equal(failed.error.effectState,'unknown');
    assert.equal(failed.error.method,script?'process.capture':'process.run');assert.equal(failed.calls.length,1);
    const notify=await invoke(script,input,{notifyFailure:{code:'ui_error',effectState:'not_started'}});
    assert.equal(notify.error.code,'ui_error');assert.equal(notify.error.effectState,'unknown');
    assert.equal(notify.error.method,'ui.notify');assert.equal(notify.calls.length,2);
    for(const value of [failed,notify])assert.equal(JSON.stringify(value.error).includes('私有'),false);
  }
});

test('严格解码错误与合法替换字符有不同结果，不能把双入口摘要相同当作字节等价',async()=>{
  const rejected=await invoke(true,input,{failure:{code:'process_invalid_utf8',effectState:'unknown'}});
  assert.equal(rejected.error.code,'process_invalid_utf8');assert.equal(rejected.calls.length,1);
  // 模拟 Node 已完成替换解码的字符串；此处不验证 Node 宿主的原始字节解码。
  const result={code:0,stdout:'包含\ufffd字符',stderr:''};
  const node=await invoke(false,input,{result});const script=await invoke(true,input,{result});
  assert.deepEqual(script.value,node.value);
  assert.equal(node.calls[1].params.message,result.stdout);
  assert.equal(script.calls[1].params.message,result.stdout);
});
