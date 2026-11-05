// 共享factory+纯内存invoke，不启动App/宿主/音频设备。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
const factory=runInNewContext(readFileSync(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8')+'\ncreateDesktopApi');
const method='audio.volume_adjust';
const value=()=>({effectState:'unknown',previousPercent:40,requestedPercent:45,currentPercent:44});
const plain=value=>JSON.parse(JSON.stringify(value));
const fault=(code,effect)=>error=>error.code===code&&error.effectState===effect&&error.method===method&&!error.message.includes('SYNTHETIC_DEVICE');
test('Node及同步公开入口以同一wire单次发送显式正负边界参数并保留实际读回',async()=>{
  for(const async of [false,true])for(const delta of [-100,-5,1,5,100]){
    let calls=0;const reply=value();reply.requestedPercent=new Map([[-100,0],[-5,35],[1,41],[5,45],[100,100]]).get(delta);
    const api=factory((wire,params)=>{calls++;assert.equal(wire,method);assert.deepEqual(plain(params),{delta_percent:delta});return async?Promise.resolve(reply):reply;});
    const pending=api.desktop.adjustVolume({deltaPercent:delta});assert.equal(typeof pending?.then==='function',async);
    assert.deepEqual(plain(await pending),reply);assert.equal(calls,1);
  }
});
test('严格范围、零值、错类型、getter及多余参数均在invoke前拒绝',()=>{
  let calls=0;const api=factory(()=>{calls++;return value();});
  const getter={get deltaPercent(){throw Error('SYNTHETIC_DEVICE');}};
  for(const args of [[],[{}],[null],[[]],[{deltaPercent:0}],[{deltaPercent:101}],[{deltaPercent:-101}],
    [{deltaPercent:1.5}],[{deltaPercent:NaN}],[{deltaPercent:Infinity}],[{deltaPercent:'5'}],[{deltaPercent:true}],
    [{deltaPercent:5,deviceId:'SYNTHETIC_DEVICE'}],[{deltaPercent:5},{}],[getter]]){
    assert.throws(()=>api.desktop.adjustVolume(...args),fault('invalid_input','not_started'));
  }assert.equal(calls,0);
});
test('额外标识、非法数值、不一致方向/效果状态被拒绝，夹紧仍为写入',async()=>{
  for(const mutate of [r=>r.currentPercent=101,r=>r.requestedPercent=null,r=>r.previousPercent=0.5,
    r=>r.device='SYNTHETIC_DEVICE',r=>r.effectState='not_started',r=>r.requestedPercent=30,
    r=>Object.defineProperty(r,'currentPercent',{get(){throw Error('SYNTHETIC_DEVICE');}})]){
    const reply=value();mutate(reply);
    for(const async of [false,true]){const api=factory(()=>async?Promise.resolve(reply):reply);
      if(async)await assert.rejects(api.desktop.adjustVolume({deltaPercent:5}),fault('invalid_response','unknown'));
      else assert.throws(()=>api.desktop.adjustVolume({deltaPercent:5}),fault('invalid_response','unknown'));
    }
  }
  const clamped={effectState:'unknown',previousPercent:100,requestedPercent:100,currentPercent:100};
  assert.deepEqual(plain(factory(()=>clamped).desktop.adjustVolume({deltaPercent:5})),clamped);
});
test('取消和写后未知保留效果阶段、原异常不泄漏且无重试',async()=>{
  for(const async of [false,true])for(const [code,effectState,expectedEffect=effectState]of[['cancelled','not_started'],['timeout','unknown'],['audio_unknown','unknown'],['audio_unknown','not_started','unknown'],['audio_failed','unknown']]){
    let calls=0;const error=Object.assign(Error('SYNTHETIC_DEVICE'),{code,effectState});
    const api=factory(()=>{calls++;if(async)return Promise.reject(error);throw error;});
    if(async)await assert.rejects(api.desktop.adjustVolume({deltaPercent:-5}),fault(code,expectedEffect));
    else assert.throws(()=>api.desktop.adjustVolume({deltaPercent:-5}),fault(code,expectedEffect));
    assert.equal(calls,1);
  }
  let reads=0;const error=Error('SYNTHETIC_DEVICE');Object.defineProperty(error,'code',{get(){reads++;return'audio_unknown';}});
  assert.throws(()=>factory(()=>{throw error;}).desktop.adjustVolume({deltaPercent:5}),fault('native_bridge_failed','unknown'));assert.equal(reads,0);
});
