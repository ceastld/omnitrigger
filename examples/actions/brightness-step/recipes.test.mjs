// 仅内存RecordingHost与共享公开SDK；不启动脚本宿主、网络、设备或第三方程序。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
import main from './main.mjs';
import {main as scriptMain} from './main-script.mjs';
const factory=runInNewContext(readFileSync(new URL('../../../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8')+'\ncreateDesktopApi');
const plain=value=>JSON.parse(JSON.stringify(value));
const method='display.brightness_step';
function receipt(strategy){
  const physical={members:[{index:0,status:'confirmed',previousRaw:40,requestedRaw:45,currentRaw:45,errorCode:null}],errorCode:null};
  const wmi={status:'zero_skipped',previousPercent:2,requestedPercent:null,currentPercent:2,errorCode:null};
  return{strategy,effectState:strategy==='wmi_first'?'not_started':'unknown',physical:strategy==='wmi_first'?null:physical,wmi:strategy==='wmi_first'?wmi:null};
}
class RecordingHost{
  constructor(asynchronous,value,error){this.asynchronous=asynchronous;this.value=value;this.error=error;this.calls=[];}
  invoke(method,params){
    this.calls.push({method,params:plain(params)});
    const finish=()=>{if(this.error)throw this.error;return this.value;};
    return this.asynchronous?Promise.resolve().then(finish):finish();
  }
}
async function run(asynchronous,input,value=receipt(input?.strategy),error){
  const host=new RecordingHost(asynchronous,value,error);
  const sdk=factory((method,params)=>host.invoke(method,params));
  const ctx={input,desktop:sdk.desktop};
  try{return{output:asynchronous?await main(ctx):scriptMain(ctx),calls:host.calls};}
  catch(error){return{error,calls:host.calls};}
}
test('两个ActionCall均明确指定输入且与清单入口和版本一致',()=>{
  const manifest=JSON.parse(readFileSync(new URL('./action.json',import.meta.url),'utf8'));
  for(const file of ['call.json','call-script.json']){
    const call=JSON.parse(readFileSync(new URL(file,import.meta.url),'utf8'));
    assert.equal(call.package_id,manifest.id);assert.equal(call.action_version,1);
    const action=manifest.actions.find(action=>action.id===call.action_id);assert(action);
    assert.equal(action.version,call.action_version);assert.equal(action.single_instance,true);
    assert.deepEqual(Object.keys(call.input).sort(),['direction','strategy']);
  }
});
for(const asynchronous of [false,true]){
  const mode=asynchronous?'Node':'同步';
  test(`${mode}旧宿主缺少方法时在读取输入前给出结构化错误`,async()=>{
    let reads=0;
    for(const desktop of [undefined,{}, {stepBrightness:null}, {stepBrightness:'unavailable'}]){
      const ctx={desktop,get input(){reads++;throw Error('输入不应被读取');}};
      const fault=error=>error.code==='runtime_unavailable'&&error.effectState==='not_started'&&error.method===method;
      if(asynchronous)await assert.rejects(main(ctx),fault);
      else assert.throws(()=>scriptMain(ctx),fault);
    }
    assert.equal(reads,0);
  });
  test(`${mode}每种显式策略只发出一次原请求并保留匿名逐项回执`,async()=>{
    for(const strategy of ['physical','wmi_first','physical_then_wmi'])for(const direction of ['up','down']){
      const value=receipt(strategy);const result=await run(asynchronous,{direction,strategy},value);
      assert.equal(result.error,undefined);assert.deepEqual(plain(result.output),value);
      assert.deepEqual(result.calls,[{method,params:{direction,strategy}}]);
    }
  });
  test(`${mode}缺参错值及额外字段不被入口过滤或默认填充`,async()=>{
    for(const input of [null,[],{}, {direction:'up'}, {strategy:'physical'}, {direction:'UP',strategy:'physical'},
      {direction:'down',strategy:'auto'},{direction:'up',strategy:'physical',retry:true}]){
      const result=await run(asynchronous,input);assert.equal(result.error.code,'invalid_input');
      assert.equal(result.error.effectState,'not_started');assert.equal(result.calls.length,0);
    }
  });
  test(`${mode}取消和写后未知只传播一次无重试且不泄漏原异常`,async()=>{
    for(const [code,effectState]of[['cancelled','not_started'],['cancelled','unknown'],['timeout','unknown']]){
      const error=Object.assign(Error('SYNTHETIC_DEVICE_ONLY'),{code,effectState});
      const result=await run(asynchronous,{direction:'up',strategy:'physical'},undefined,error);
      assert.equal(result.error.code,code);assert.equal(result.error.effectState,effectState);
      assert.equal(result.error.method,method);assert(!result.error.message.includes('SYNTHETIC_DEVICE_ONLY'));assert.equal(result.calls.length,1);
    }
  });
  test(`${mode}部分未知、零档和畸形回执均复用SDK契约不另猜成功`,async()=>{
    const unknown=receipt('physical');unknown.physical.members[0]={index:0,status:'set_unconfirmed',previousRaw:40,requestedRaw:45,currentRaw:null,errorCode:'brightness_unknown'};
    const partial=await run(asynchronous,{direction:'up',strategy:'physical'},unknown);assert.deepEqual(plain(partial.output),unknown);
    const zero=await run(asynchronous,{direction:'down',strategy:'wmi_first'});assert.equal(zero.output.wmi.status,'zero_skipped');
    const bad=receipt('physical');bad.physical.members[0].deviceId='SYNTHETIC_IDENTIFIER';
    const malformed=await run(asynchronous,{direction:'up',strategy:'physical'},bad);assert.equal(malformed.error.code,'invalid_response');assert.equal(malformed.error.effectState,'unknown');assert.equal(malformed.calls.length,1);
  });
}
