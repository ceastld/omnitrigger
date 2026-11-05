// 内存 invoke + 同一公开 factory；无宿主、网络、硬件或个人数据。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
const source=readFileSync(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const factory=runInNewContext(source+'\ncreateDesktopApi');
const plain=value=>JSON.parse(JSON.stringify(value));
const method='display.brightness_step';
const secret='PRIVATE_DEVICE_IDENTIFIER_AND_EXCEPTION';
const member=(status='confirmed')=>({index:0,status,previousRaw:40,requestedRaw:45,currentRaw:45,errorCode:null});
const physical=()=>({members:[member()],errorCode:null});
const wmi=()=>({status:'zero_skipped',previousPercent:2,requestedPercent:null,currentPercent:2,errorCode:null});
const reply=strategy=>({strategy,effectState:strategy==='wmi_first'?'not_started':'unknown',physical:strategy==='wmi_first'?null:physical(),wmi:strategy==='physical'?null:wmi()});
const rejected=(code,effect)=>error=>error.code===code&&error.effectState===effect&&error.method===method&&!error.message.includes(secret);
test('同步和await入口对三种显式策略只调用同一wire一次并返回新匿名对象',async()=>{
  for(const asynchronous of [false,true])for(const strategy of ['physical','wmi_first','physical_then_wmi']){
    const value=reply(strategy);if(strategy==='physical_then_wmi')value.wmi=null;let calls=0;
    const api=factory((wire,params)=>{calls++;assert.equal(wire,method);assert.deepEqual(plain(params),{direction:'down',strategy});return asynchronous?Promise.resolve(value):value;});
    const pending=api.desktop.stepBrightness({direction:'down',strategy});assert.equal(typeof pending?.then==='function',asynchronous);
    const result=await pending;assert.deepEqual(plain(result),value);assert.notEqual(result,value);assert.equal(calls,1);
  }
});
test('参数严格必填，不默认策略；数组、额外字段、符号、getter、缺参均在invoke前拒绝',()=>{
  let calls=0;const api=factory(()=>{calls++;throw Error('不得调用');});
  const getter={strategy:'physical',get direction(){throw Error(secret);}};
  for(const args of [[],[undefined],[null],[[]],[{}],[{direction:'up'}],[{direction:'up',strategy:null}],
    [{direction:'UP',strategy:'physical'}],[{direction:'up',strategy:'physical',retry:true}],
    [{direction:'up',strategy:'physical',[Symbol()]:true}],[getter],[{direction:'up',strategy:'physical'},{}]]){
    assert.throws(()=>api.desktop.stepBrightness(...args),rejected('invalid_input','not_started'));
  }assert.equal(calls,0);
});
test('部分、未知和零档回执保持结构，错误正文和标识不能夹带通过',async()=>{
  const value={strategy:'physical_then_wmi',effectState:'not_started',physical:{members:[{index:0,status:'read_rejected',previousRaw:null,requestedRaw:null,currentRaw:null,errorCode:'brightness_failed'}],errorCode:null},wmi:wmi()};
  for(const async of [false,true]){
    const api=factory(()=>async?Promise.resolve(value):value);assert.deepEqual(plain(await api.desktop.stepBrightness({direction:'down',strategy:value.strategy})),value);
  }
  const unknown={strategy:'physical',effectState:'unknown',physical:{members:[{index:0,status:'set_unconfirmed',previousRaw:40,requestedRaw:45,currentRaw:null,errorCode:'brightness_unknown'}],errorCode:null},wmi:null};
  assert.deepEqual(plain(factory(()=>unknown).desktop.stepBrightness({direction:'up',strategy:'physical'})),unknown);
});
test('逐项及总回执非法时拒绝，不能错误回退或将不符读回当确认',async()=>{
  const cases=[];
  for(const mutate of [v=>v.device=secret,v=>v.physical.members[0].label=secret,v=>v.physical.members[0].currentRaw=44,
    v=>v.physical.members[0].index=1,v=>v.physical.members[0].previousRaw=-1,v=>v.physical.members[0].status='done',
    v=>v.physical.members[0].errorCode=secret,v=>v.physical.members.push(member()),v=>v.physical.members=[],
    v=>v.physical.members[0].currentRaw=2**32,v=>v.wmi=wmi(),v=>v.effectState='not_started']){const value=reply('physical');mutate(value);cases.push(value);}
  cases.push(reply('physical_then_wmi')); // 已确认DDC写入后不得回退WMI。
  const sparse=reply('physical');delete sparse.physical.members[0];cases.push(sparse);
  const accessor=reply('physical');Object.defineProperty(accessor.physical.members[0],'currentRaw',{get(){throw Error(secret);}});cases.push(accessor);
  for(const value of cases)for(const asynchronous of [false,true]){
    const api=factory(()=>asynchronous?Promise.resolve(value):value);
    if(asynchronous)await assert.rejects(api.desktop.stepBrightness({direction:'up',strategy:value.strategy}),rejected('invalid_response','unknown'));
    else assert.throws(()=>api.desktop.stepBrightness({direction:'up',strategy:value.strategy}),rejected('invalid_response','unknown'));
  }
});
test('取消期限和桥接失败保持效果边界，不泄漏原异常且不重试',async()=>{
  for(const asynchronous of [false,true])for(const [code,effectState]of[['cancelled','not_started'],['timeout','unknown'],['brightness_unknown','unknown']]){
    let calls=0;const error=Object.assign(Error(secret),{code,effectState});
    const api=factory(()=>{calls++;if(asynchronous)return Promise.reject(error);throw error;});
    if(asynchronous)await assert.rejects(api.desktop.stepBrightness({direction:'up',strategy:'physical'}),rejected(code,effectState));
    else assert.throws(()=>api.desktop.stepBrightness({direction:'up',strategy:'physical'}),rejected(code,effectState));
    assert.equal(calls,1);
  }
  assert.throws(()=>factory(()=>{throw Object.assign(Error(secret),{code:secret});}).desktop.stepBrightness({direction:'up',strategy:'physical'}),rejected('native_bridge_failed','unknown'));
});
test('WMI值范围/零档约束和DDC明确写前失败才允许的回退被复核',()=>{
  const value={strategy:'physical_then_wmi',effectState:'not_started',physical:{members:[],errorCode:'brightness_unavailable'},wmi:wmi()};
  assert.deepEqual(plain(factory(()=>value).desktop.stepBrightness({direction:'down',strategy:value.strategy})),value);
  for(const change of [v=>v.wmi.requestedPercent=0,v=>v.wmi.currentPercent=101,v=>v.wmi.instance=secret,v=>v.physical.errorCode='brightness_unknown']){
    const bad=structuredClone(value);change(bad);
    assert.throws(()=>factory(()=>bad).desktop.stepBrightness({direction:'down',strategy:bad.strategy}),rejected('invalid_response','unknown'));
  }
});
