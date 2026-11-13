// 测试正式SDK工厂及两轻量引擎共用的sdk.js；宿主仅记录JSON，不读取UIA或启动进程。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
const runtime=new URL('../crates/omnitrigger-actions/runtime/',import.meta.url);
const desktopSource=readFileSync(new URL('desktop-api.js',runtime),'utf8');
const sharedSource=['desktop-api.js','browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js']
  .map(name=>readFileSync(new URL(name,runtime),'utf8')).join('\n')+'\n'
  +readFileSync(new URL('../tools/js-engine-probe/src/sdk.js',import.meta.url),'utf8');
const plain=value=>JSON.parse(JSON.stringify(value));
const original={window_handle:'101',process_id:10,thread_id:11,process_created_file_time:'134000000000000001',expires_at_unix_ms:1000};
function api(invoke){return vm.runInNewContext(desktopSource+'\ncreateDesktopApi(invoke).desktop',{invoke});}
const matches=code=>error=>error.code===code&&error.method==='selection.read_text'
  &&error.effectState==='not_started'&&!error.message.includes('PRIVATE');
for(const asynchronous of [false,true]){
  test(`${asynchronous?'异步':'同步'}空成功与非空成功均为冻结快照，保持原目标及单一wire`,async()=>{
    const calls=[];let raw={text:'',truncated:false};
    const desktop=api((method,params)=>{calls.push({method,params:plain(params)});const value=method==='target.context'?original:raw;return asynchronous?Promise.resolve(value):value;});
    const before=await desktop.targetContext();const empty=await desktop.readSelectionText();
    assert.deepEqual(plain(empty),{text:'',truncated:false});assert(Object.isFrozen(empty));
    raw.text='changed';assert.equal(empty.text,'');
    raw={text:'  合成😀\r\n',truncated:false};const text=await desktop.readSelectionText({overflow:'reject'});
    assert.equal(text.text,raw.text);assert(Object.isFrozen(text));
    assert.deepEqual(plain(await desktop.targetContext()),plain(before));
    assert.deepEqual(calls,[{method:'target.context',params:{}},{method:'selection.read_text',params:{}},
      {method:'selection.read_text',params:{overflow:'reject'}},{method:'target.context',params:{}}]);
  });
  test(`${asynchronous?'异步':'同步'}provider/目标/取消失败保持错误且不补空、不重试`,async()=>{
    for(const code of ['selection_unavailable','target_changed','selection_changed','timeout','cancelled']){
      let calls=0;const desktop=api(()=>{calls++;const error=Object.assign(Error('PRIVATE provider text'),{code});if(asynchronous)return Promise.reject(error);throw error;});
      if(asynchronous)await assert.rejects(desktop.readSelectionText(),matches(code));else assert.throws(()=>desktop.readSelectionText(),matches(code));
      assert.equal(calls,1);
    }
  });
}
test('默认reject及显式truncate准确传参；空值不允许truncated=true',async()=>{
  const calls=[];let reply={text:'A'.repeat(128),truncated:true};
  const desktop=api((method,params)=>{calls.push({method,params:plain(params)});return Promise.resolve(reply);});
  await assert.rejects(desktop.readSelectionText(),matches('invalid_response'));
  const options={overflow:'truncate'};const pending=desktop.readSelectionText(options);options.overflow='reject';
  assert.equal((await pending).text,'A'.repeat(128));assert.deepEqual(calls.at(-1).params,{overflow:'truncate'});
  reply={text:'',truncated:false};assert.deepEqual(plain(await desktop.readSelectionText({overflow:'truncate'})),reply);
  reply={text:'',truncated:true};await assert.rejects(desktop.readSelectionText({overflow:'truncate'}),matches('invalid_response'));
});
test('实际工厂严格检查128单元Unicode及回执形态，不截断或有损修复宿主数据',()=>{
  for(const text of ['字'.repeat(128),'😀'.repeat(64),'\r\n'.repeat(64)]){
    const reply={text,truncated:false};assert.deepEqual(plain(api(()=>reply).readSelectionText()),reply);
  }
  for(const value of [{text:'A'.repeat(129),truncated:false},{text:'\ud800',truncated:false},
    {text:'\udc00',truncated:false},{text:'A'.repeat(127)+'\ud800',truncated:true},
    {text:'\0',truncated:false},{text:'short',truncated:true},{text:''},{text:'',truncated:false,extra:true},null]){
    assert.throws(()=>api(()=>value).readSelectionText({overflow:'truncate'}),matches('invalid_response'));
  }
});
test('无效选项在任何宿主调用前拒绝',()=>{
  let calls=0;const desktop=api(()=>{calls++;return{text:'',truncated:false};});
  for(const options of [null,[],{overflow:'lossy'},{overflow:null},{maxLength:128},{allowEmpty:true},{[Symbol('extra')]:true}]){
    assert.throws(()=>desktop.readSelectionText(options),matches('invalid_input'));
  }
  assert.throws(()=>desktop.readSelectionText({},{}),matches('invalid_input'));assert.equal(calls,0);
});
function shared(error){
  const requests=[];const scope={ctx:{input:{},invocation:{invocationId:'1'},__nativeContext:{connection:{port:1,token:'synthetic'}}},
    __hostCall:serialized=>{const request=JSON.parse(serialized);requests.push(request);
      const value=request.kind==='native'?(request.method==='target.context'?original:{text:'',truncated:false}):null;
      return JSON.stringify(error&&request.kind==='native'?{ok:false,error}:{ok:true,value});}};
  vm.runInNewContext(sharedSource,scope);return{ctx:scope.ctx,requests};
}
test('实际共享sdk.js的选区、原上下文和delay合计32次，不因overflow重置',()=>{
  const h=shared();h.ctx.delay(0);for(let i=0;i<30;i++)h.ctx.desktop.targetContext();
  assert.deepEqual(plain(h.ctx.desktop.readSelectionText()),{text:'',truncated:false});assert.equal(h.requests.length,32);
  assert.equal(Object.hasOwn(h.ctx,'__nativeContext'),false);
  assert.throws(()=>h.ctx.desktop.readSelectionText({overflow:'truncate'}));assert.equal(h.requests.length,32);
  assert(h.requests.filter(r=>r.kind==='native').every(r=>['target.context','selection.read_text'].includes(r.method)));
});
test('共享SDK错误同样计入32总预算，不复制、读取Clipboard或自动重试',()=>{
  const h=shared({code:'selection_unavailable',message:'PRIVATE',effect_state:'not_started'});
  assert.throws(()=>h.ctx.desktop.readSelectionText(),matches('selection_unavailable'));
  for(let i=0;i<31;i++)h.ctx.delay(0);
  assert.throws(()=>h.ctx.desktop.readSelectionText());assert.equal(h.requests.length,32);
  assert.equal(h.requests.filter(r=>r.kind==='native').length,1);
});
