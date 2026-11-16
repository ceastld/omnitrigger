import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
const source=readFileSync(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const factory=vm.runInNewContext(source+'\ncreateDesktopApi');
const snapshot=()=>({window_handle:'9007199254740993',process_id:11,thread_id:12,process_created_file_time:'18446744073709550000',expires_at_unix_ms:Date.now()+4900});
function fixture(mode,options={}){
  const calls=[];
  const invoke=(method,params)=>{
    calls.push({method,params});
    const run=()=>{
      if(method==='raishot.resident.find')return options.find===undefined?snapshot():options.find;
      if(options.error)throw options.error;
      return options.reply??{queued:true,screenshot_completed:false};
    };
    return mode==='node'?Promise.resolve().then(run):run();
  };
  const api=factory(invoke).desktop;
  return {api,calls};
}
for(const mode of ['script','node']){
  test(mode+': 原身份精度、一次请求和入队限定回执',async()=>{
    const f=fixture(mode);const value=await f.api.findRaiShotResident();
    assert.equal(Object.isFrozen(value),true);assert.equal(value.window_handle,'9007199254740993');
    const reply=await f.api.requestRaiShotRegion(value);
    assert.equal(reply.queued,true);assert.equal(reply.screenshotCompleted,false);
    assert.deepEqual(JSON.parse(JSON.stringify(f.calls)),JSON.parse(JSON.stringify([{method:'raishot.resident.find',params:{}},{method:'raishot.resident.region',params:{resident:value}}])));
    await assert.rejects(async()=>f.api.requestRaiShotRegion(value),e=>e.code==='invalid_input'&&e.effectState==='not_started');
    assert.equal(f.calls.length,2);
  });
  test(mode+': 没有常驻返回null，不启动进程',async()=>{
    const f=fixture(mode,{find:null});assert.equal(await f.api.findRaiShotResident(),null);assert.equal(f.calls.length,1);
  });
  test(mode+': 伪造或另一个ctx快照在派发前拒绝',async()=>{
    const a=fixture(mode),b=fixture(mode);const value=await a.api.findRaiShotResident();
    for(const item of [{...value},value])await assert.rejects(async()=>b.api.requestRaiShotRegion(item),e=>e.code==='invalid_input'&&e.effectState==='not_started');
    assert.equal(b.calls.length,0);
  });
  test(mode+': 非法快照回执拒绝',async()=>{
    const f=fixture(mode,{find:{...snapshot(),window_handle:'0'}});
    await assert.rejects(async()=>f.api.findRaiShotResident(),e=>e.code==='invalid_response'&&e.effectState==='not_started');
  });
  test(mode+': 部分/伪成功回执保持unknown，不重试',async()=>{
    const f=fixture(mode,{reply:{queued:true,screenshot_completed:true}});const value=await f.api.findRaiShotResident();
    await assert.rejects(async()=>f.api.requestRaiShotRegion(value),e=>e.code==='invalid_response'&&e.effectState==='unknown'&&e.retryAllowed===false);
    assert.equal(f.calls.length,2);
  });
  test(mode+': 窗口变化的明确未提交保持not_started且不暴露私有错误',async()=>{
    const f=fixture(mode,{error:{code:'raishot_resident_changed',effectState:'not_started',message:'PRIVATE'}});const value=await f.api.findRaiShotResident();
    await assert.rejects(async()=>f.api.requestRaiShotRegion(value),e=>e.code==='raishot_resident_changed'&&e.effectState==='not_started'&&!e.message.includes('PRIVATE'));
    assert.equal(f.calls.length,2);
  });
}
test('真实同步SDK的native桥接：写出后缺效果字段保持unknown，错误正文不回显',()=>{
  const calls=[];
  const sdk=readFileSync(new URL('../crates/omnitrigger-actions/runtime/actions-api.js',import.meta.url),'utf8')+'\n'+readFileSync(new URL('../crates/omnitrigger-actions/runtime/state-api.js',import.meta.url),'utf8')+'\n'+readFileSync(new URL('../tools/js-engine-probe/src/sdk.js',import.meta.url),'utf8');
  const context=vm.createContext({ctx:{input:{},__nativeContext:{connection:{port:9,token:'a'.repeat(64)}}},
    createProcessCaptureApi:()=>({capture(){}}),createBrowserApi:()=>({}),createClipboardHistoryApi:()=>({}),
    __hostCall(raw){const request=JSON.parse(raw);calls.push(request);
      return JSON.stringify(request.method==='raishot.resident.find'?{ok:true,value:snapshot()}:{ok:false,error:{code:'timeout',message:'PRIVATE'}});}});
  vm.runInContext(source+'\n'+sdk,context);
  const value=context.ctx.desktop.findRaiShotResident();
  assert.throws(()=>context.ctx.desktop.requestRaiShotRegion(value),e=>e.code==='timeout'&&e.effectState==='unknown'&&e.method==='raishot.resident.region'&&!e.message.includes('PRIVATE'));
  assert.deepEqual(calls.map(x=>x.method),['raishot.resident.find','raishot.resident.region']);
  assert.equal(calls.every(x=>x.kind==='native'),true);
});
