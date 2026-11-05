// 仅RecordingHost和共享SDK，不连接设备、App、网络或CLI。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
import nodeMain from './main.mjs';
import {main as scriptMain} from './main-script.mjs';
const factory=runInNewContext(readFileSync(new URL('../../../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8')+'\ncreateDesktopApi');
const method='audio.volume_adjust';const plain=value=>JSON.parse(JSON.stringify(value));
class RecordingHost{
  constructor(async,value,error){this.async=async;this.value=value;this.error=error;this.calls=[];}
  invoke(method,params){this.calls.push({method,params:plain(params)});const finish=()=>{if(this.error)throw this.error;return this.value;};return this.async?Promise.resolve().then(finish):finish();}
}
async function call(async,input,{reply={effectState:'unknown',previousPercent:40,requestedPercent:45,currentPercent:44},error,desktop}={}){
  const host=new RecordingHost(async,reply,error),api=factory((method,params)=>host.invoke(method,params));
  const ctx={input,desktop:desktop===undefined?api.desktop:desktop};
  try{return{value:async?await nodeMain(ctx):scriptMain(ctx),calls:host.calls};}catch(error){return{error,calls:host.calls};}
}
test('Node和同步均显式传递单次百分点调用并保留实际匿名读回',async()=>{
  for(const async of [false,true])for(const deltaPercent of [5,-5]){
    const reply={effectState:'unknown',previousPercent:40,requestedPercent:deltaPercent>0?45:35,currentPercent:44};
    const result=await call(async,{deltaPercent},{reply});assert.deepEqual(plain(result.value),reply);
    assert.deepEqual(result.calls,[{method,params:{delta_percent:deltaPercent}}]);
  }
});
test('缺API先于输入检查返回runtime_unavailable且不调用其他接口',async()=>{
  for(const async of [false,true])for(const desktop of [undefined,{},null,{adjustVolume:0}]){
    let inputReads=0;const ctx={desktop,get input(){inputReads++;throw Error('不得读取输入');}};
    let observed;try{if(async)await nodeMain(ctx);else scriptMain(ctx);}catch(error){observed=error;}
    assert.equal(observed.code,'runtime_unavailable');assert.equal(observed.effectState,'not_started');assert.equal(observed.method,method);assert.equal(inputReads,0);
  }
});
test('必填及未知字段由SDK拒绝，不补默认或过滤输入',async()=>{
  for(const async of [false,true])for(const input of [{},{deltaPercent:0},{deltaPercent:'5'},{deltaPercent:5,strategy:'physical'}]){
    const result=await call(async,input);assert.equal(result.error.code,'invalid_input');assert.equal(result.error.effectState,'not_started');assert.equal(result.calls.length,0);
  }
});
test('取消及写后未知原样保守传播，不重试或泄漏异常正文',async()=>{
  for(const async of [false,true])for(const [code,effectState,expected]of[['cancelled','not_started','not_started'],['timeout','unknown','unknown'],['audio_unknown','not_started','unknown']]){
    const error=Object.assign(Error('SYNTHETIC_AUDIO_DEVICE'),{code,effectState});const result=await call(async,{deltaPercent:5},{error});
    assert.equal(result.error.code,code);assert.equal(result.error.effectState,expected);assert(!result.error.message.includes('SYNTHETIC_AUDIO_DEVICE'));assert.equal(result.calls.length,1);
  }
});
test('两个ActionCall与清单匹配，方向值是显式输入而非默认',()=>{
  const manifest=JSON.parse(readFileSync(new URL('./action.json',import.meta.url),'utf8'));
  for(const name of ['call.json','call-script.json']){const value=JSON.parse(readFileSync(new URL(name,import.meta.url),'utf8'));
    assert.equal(value.package_id,manifest.id);const action=manifest.actions.find(action=>action.id===value.action_id);assert(action);assert.equal(value.action_version,action.version);
    assert.deepEqual(Object.keys(value.input),['deltaPercent']);assert([5,-5].includes(value.input.deltaPercent));
  }
});
