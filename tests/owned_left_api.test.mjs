import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
const source=readFileSync(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
function api(invoke){return vm.runInNewContext(source+'\ncreateDesktopApi(invoke).desktop',{invoke});}
for(const asynchronous of [false,true]){
 test((asynchronous?'Promise':'sync')+' owned left API uses exact empty requests and strict receipts',async()=>{
  const calls=[];const desktop=api((method,params)=>{calls.push([method,JSON.parse(JSON.stringify(params))]);const value=method==='mouse.left_down'?{submitted:true,effectState:'unknown'}:{ownedPressEnded:true,effectState:'unknown'};return asynchronous?Promise.resolve(value):value;});
  const down=await desktop.pressLeftButton(),up=await desktop.releaseLeftButton();
  assert(Object.isFrozen(down)&&Object.isFrozen(up));
  assert.deepEqual(calls,[['mouse.left_down',{}],['mouse.left_up',{}]]);
 });
}
test('owned left options/forged owner rejected before bridge',()=>{
 let calls=0;const desktop=api(()=>{calls++;throw Error('must not invoke');});
 for(const call of [()=>desktop.pressLeftButton({owner:1}),()=>desktop.releaseLeftButton({}),()=>desktop.pressLeftButton(undefined)]){
  assert.throws(call,error=>error.code==='invalid_input'&&error.effectState==='not_started');
 }
 assert.equal(calls,0);
});
test('missing owner channel and malformed receipts stay explicit failures',async()=>{
 const unavailable=api(()=>{throw Object.assign(Error('PRIVATE target data'),{code:'input_owner_unavailable',effectState:'not_started'});});
 assert.throws(()=>unavailable.pressLeftButton(),error=>error.code==='input_owner_unavailable'&&error.method==='mouse.left_down'&&!error.message.includes('PRIVATE'));
 for(const result of [{submitted:false,effectState:'unknown'},{submitted:true,effectState:'not_started'},{submitted:true,effectState:'unknown',extra:true}]){
  const desktop=api(()=>Promise.resolve(result));
  await assert.rejects(desktop.pressLeftButton(),error=>error.code==='invalid_response'&&error.effectState==='unknown');
 }
});


test('owned-left故障元数据不读取getter，清理失败不能变成未开始',()=>{
  for(const method of ['pressLeftButton','releaseLeftButton']){
    let calls=0,getters=0;
    const error=Object.assign(new Error('PRIVATE'),{code:'input_cleanup_failed',effectState:'not_started'});
    assert.throws(()=>api(()=>{calls++;throw error;})[method](),fault=>fault.code==='input_cleanup_failed'&&fault.effectState==='unknown'&&!fault.message.includes('PRIVATE'));
    assert.equal(calls,1);
    for(const field of ['code','effectState']){
      const cause={};Object.defineProperty(cause,field,{get(){getters++;throw Error('PRIVATE');}});
      assert.throws(()=>api(()=>{throw cause;})[method](),fault=>fault.effectState==='unknown');
    }
    assert.equal(getters,0);
  }
});
