// 只用共享factory与内存invoke；不调用Host、网络、设备或个人数据。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
const factory=runInNewContext(readFileSync(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8')+'\ncreateDesktopApi');
const privateMarker='SYNTHETIC_IDENTIFIER_MUST_NOT_ESCAPE';
const method='display.brightness_step';
const opts={direction:'up',strategy:'physical'};
const member=()=>({index:0,status:'confirmed',previousRaw:40,requestedRaw:45,currentRaw:45,errorCode:null});
const physical=()=>({strategy:'physical',effectState:'unknown',physical:{members:[member()],errorCode:null},wmi:null});
const zero=()=>({status:'zero_skipped',previousPercent:2,requestedPercent:null,currentPercent:2,errorCode:null});
const fault=(code,effect)=>error=>error.code===code&&error.method===method&&error.effectState===effect
  &&!String(error.message).includes(privateMarker)&&!String(error.code).includes(privateMarker);
const normalize=value=>JSON.parse(JSON.stringify(value));
async function invalid(value,options=opts){
  for(const async of [false,true]){
    const api=factory(()=>async?Promise.resolve(value):value);
    if(async)await assert.rejects(api.desktop.stepBrightness(options),fault('invalid_response','unknown'));
    else assert.throws(()=>api.desktop.stepBrightness(options),fault('invalid_response','unknown'));
  }
}
test('异常getter不被执行，错误代码只能来自一次自有数据属性快照',async()=>{
  for(const async of [false,true]){
    let reads=0;const error=Error(privateMarker);
    Object.defineProperty(error,'code',{get(){return ++reads===1?'timeout':privateMarker;}});
    Object.defineProperty(error,'effectState',{get(){reads++;return 'not_started';}});
    const api=factory(()=>{if(async)return Promise.reject(error);throw error;});
    if(async)await assert.rejects(api.desktop.stepBrightness(opts),fault('native_bridge_failed','unknown'));
    else assert.throws(()=>api.desktop.stepBrightness(opts),fault('native_bridge_failed','unknown'));
    assert.equal(reads,0);
  }
});
test('invoke修改内部参数不能改变最初请求策略的回执校验',async()=>{
  for(const async of [false,true]){
    const api=factory((_,params)=>{
      params.strategy='wmi_first';
      const value={strategy:'wmi_first',effectState:'not_started',physical:null,wmi:zero()};
      return async?Promise.resolve(value):value;
    });
    if(async)await assert.rejects(api.desktop.stepBrightness(opts),fault('invalid_response','unknown'));
    else assert.throws(()=>api.desktop.stepBrightness(opts),fault('invalid_response','unknown'));
  }
});
test('超大数组、错误字符串、状态及结构均拒绝且不回显内容',async()=>{
  for(const count of [65,0xffffffff]){const value=physical();value.physical.members=new Array(count);await invalid(value);}
  for(const change of [v=>v.physical.members[0].errorCode=privateMarker.repeat(32768),
    v=>v.physical.members[0].status='x'.repeat(1024*1024),v=>v.physical.members[0].currentRaw=Infinity,
    v=>v.physical.members[0].currentRaw=NaN,v=>v.physical.members[0].currentRaw=45.5,
    v=>v.physical.members[0].requestedRaw=-1,v=>v.physical.members[0].previousRaw=2**32]){
    const value=physical();change(value);await invalid(value);
  }
  let calls=0;const api=factory(()=>{calls++;return physical();});
  assert.throws(()=>api.desktop.stepBrightness({direction:'x'.repeat(1024*1024),strategy:'physical'}),fault('invalid_input','not_started'));
  assert.equal(calls,0);
});
test('额外标识、符号、非枚举字段、getter、稀疏成员和错误序号拒绝',async()=>{
  for(const change of [v=>v.identifier=privateMarker,v=>v.physical.members[0].label=privateMarker,
    v=>v.physical[Symbol()]=privateMarker,v=>Object.defineProperty(v.physical,'hidden',{value:privateMarker}),
    v=>Object.defineProperty(v,'effectState',{get(){throw Error(privateMarker);}}),
    v=>delete v.physical.members[0],v=>v.physical.members[0].index=1,
    v=>v.physical.members.extra=true]){const value=physical();change(value);await invalid(value);}
});
test('已知写入或unknown失败不得标为not_started，非法效果枚举亦拒绝',async()=>{
  const confirmed=physical();confirmed.effectState='not_started';await invalid(confirmed);
  for(const effectState of [null,undefined,'confirmed','completed',0]){const value=physical();value.effectState=effectState;await invalid(value);}
  await invalid({strategy:'wmi_first',effectState:'not_started',physical:null,
    wmi:{status:'failed',previousPercent:null,requestedPercent:null,currentPercent:null,errorCode:'brightness_unknown'}},
    {direction:'up',strategy:'wmi_first'});
  await invalid({strategy:'physical',effectState:'not_started',physical:{members:[],errorCode:'brightness_unknown'},wmi:null});
});
test('正常确认与取消unknown各保留本义，WMI非精确读回不被虚构为失败',async()=>{
  for(const async of [false,true]){
    const value=physical();const api=factory(()=>async?Promise.resolve(value):value);
    assert.deepEqual(normalize(await api.desktop.stepBrightness(opts)),value);
    const stopped=factory(()=>{const error=Object.assign(Error(privateMarker),{code:'cancelled',effectState:'unknown'});if(async)return Promise.reject(error);throw error;});
    if(async)await assert.rejects(stopped.desktop.stepBrightness(opts),fault('cancelled','unknown'));
    else assert.throws(()=>stopped.desktop.stepBrightness(opts),fault('cancelled','unknown'));
  }
  const value={strategy:'wmi_first',effectState:'unknown',physical:null,wmi:{status:'set',previousPercent:40,requestedPercent:60,currentPercent:59,errorCode:null}};
  assert.deepEqual(normalize(factory(()=>value).desktop.stepBrightness({direction:'up',strategy:'wmi_first'})),value);
  const drift=physical();drift.physical.members[0].status='readback_different';
  assert.deepEqual(normalize(factory(()=>drift).desktop.stepBrightness(opts)),drift);
});
test('无写入零档正常返回，写后不能隐式转入第二协议',async()=>{
  const value={strategy:'wmi_first',effectState:'not_started',physical:null,wmi:zero()};
  assert.deepEqual(normalize(factory(()=>value).desktop.stepBrightness({direction:'down',strategy:'wmi_first'})),value);
  const bad=physical();bad.strategy='physical_then_wmi';bad.wmi=zero();await invalid(bad,{direction:'up',strategy:'physical_then_wmi'});
});
