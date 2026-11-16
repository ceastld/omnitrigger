import assert from 'node:assert/strict';
import test from 'node:test';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import nodeMain from './node.mjs';
import scriptMain from './script.mjs';
import {fileURLToPath} from 'node:url';
import cases from './contract-cases.mjs';
const ROOT=fileURLToPath(new URL('../../../',import.meta.url));
const desktop=readFileSync(ROOT+'crates/omnitrigger-actions/runtime/desktop-api.js','utf8');
const support=['browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js','state-api.js'].map(name=>readFileSync(ROOT+'crates/omnitrigger-actions/runtime/'+name,'utf8')).join('\n');
const sdk=readFileSync(ROOT+'tools/js-engine-probe/src/sdk.js','utf8');
const factory=runInNewContext(desktop+'\ncreateDesktopApi;');
const cause=(code,effectState='not_started',method='selection.read_text')=>Object.assign(Error('PRIVATE'),{code,effectState,method});
function harness(mode,spec,overrides={}){
  const calls=[];let reads=0;
  const route=(method,params)=>{
    calls.push(JSON.parse(JSON.stringify({method,params})));
    if(method==='selection.read_text'){
      reads++;
      if(spec.pauseFirstRead&&reads===2)throw cause('target_changed');
      const value=overrides.reads?.[reads-1]??{text:spec.selection==='caret'?'':spec.text,truncated:false};
      if(value instanceof Error)throw value;
      if(value.text.length>(params.max_utf16_units??128))throw cause('selection_too_long');
      return value;
    }
    assert.equal(method,'keyboard.text');assert(params.text.length<=128,'consumer must never submit over128');
    if(overrides.sendError)throw overrides.sendError;
    return {submitted:true};
  };
  let ctx;
  if(mode==='node')ctx={input:{operation:spec.operation},signal:new AbortController().signal,desktop:factory(async(method,params)=>route(method,params)).desktop};
  else{
    const scope={ctx:{input:{operation:spec.operation},__nativeContext:{connection:{port:1,token:'synthetic'}}},__hostCall:encoded=>{
      const req=JSON.parse(encoded);assert.equal(req.kind,'native');
      try{return JSON.stringify({ok:true,value:route(req.method,req.params)});}
      catch(error){return JSON.stringify({ok:false,error:{code:error.code,message:error.message,effect_state:error.effectState,method:error.method}});}
    }};
    runInNewContext(desktop+'\n'+support+'\n'+sdk,scope);ctx=scope.ctx;
  }
  return {calls,run:()=>spec.defaultLimitProbe?ctx.desktop.readSelectionText({overflow:'reject'}):mode==='node'?nodeMain(ctx):scriptMain(ctx)};
}
for(const mode of ['node','script']){
  for(const spec of cases)test(mode+'/'+spec.name,async()=>{
    const h=harness(mode,spec);
    if(spec.expectedCode)await assert.rejects(async()=>h.run(),error=>error.code===spec.expectedCode&&error.effectState==='not_started'&&error.method===(spec.expectedMethod??'selection.read_text'));
    else{
      const result=h.run();assert.equal(typeof result?.then,mode==='node'?'function':'undefined');
      assert.equal((await result).status,spec.expectedStatus);
    }
    const reads=h.calls.filter(c=>c.method==='selection.read_text'),sends=h.calls.filter(c=>c.method==='keyboard.text');
    assert.equal(reads.length,spec.reads);assert.equal(sends.length,spec.sends);
    for(const read of reads)assert.deepEqual(read.params,spec.defaultLimitProbe?{overflow:'reject'}:{overflow:'reject',max_utf16_units:8192});
    if(sends.length)assert.equal(sends[0].params.text,spec.expectedText);
  });
  test(mode+': 不变8192原文仍只读一次且不发送',async()=>{
    const h=harness(mode,{operation:'remove-empty-lines',text:'A'.repeat(8192)});
    assert.equal((await h.run()).status,'unchanged');assert.equal(h.calls.length,1);
  });
  test(mode+': 超128输出即使潜在复读会失败也不进行第二read',async()=>{
    const text='A'.repeat(127)+'\n\nB',h=harness(mode,{operation:'remove-empty-lines',text},{reads:[{text,truncated:false},cause('target_changed')]});
    await assert.rejects(async()=>h.run(),e=>e.code==='replacement_too_long'&&e.effectState==='not_started'&&e.method==='keyboard.text');
    assert.equal(h.calls.length,1);
  });
  test(mode+': 短结果复读变化仍拒绝不发',async()=>{
    const text='A\r\n\r\nB',h=harness(mode,{operation:'remove-empty-lines',text},{reads:[{text,truncated:false},{text:'C\r\n\r\nD',truncated:false}]});
    await assert.rejects(async()=>h.run(),e=>e.code==='selection_changed'&&e.effectState==='not_started');assert.equal(h.calls.length,2);
  });
  test(mode+': 短结果未知写仍只尝试一次',async()=>{
    const h=harness(mode,{operation:'remove-empty-lines',text:'A\r\n\r\nB'},{sendError:cause('input_failed','unknown','keyboard.text')});
    await assert.rejects(async()=>h.run(),e=>e.code==='input_failed'&&e.effectState==='unknown'&&e.method==='keyboard.text');assert.equal(h.calls.length,3);
  });
  test(mode+': 不允许截断源快照或拆开emoji后写',async()=>{
    for(const value of [{text:'A'.repeat(128),truncated:true},{text:'\ud83d',truncated:false}]){
      const h=harness(mode,{operation:'remove-empty-lines',text:''},{reads:[value]});
      await assert.rejects(async()=>h.run(),e=>e.code==='invalid_response'&&e.effectState==='not_started');assert.equal(h.calls.length,1);
    }
  });
}
test('输出限制属于consumer，真实SDK既有129发送能力未更改',()=>{
  const calls=[],api=factory((method,params)=>{calls.push(JSON.parse(JSON.stringify({method,params})));return{submitted:true};});
  api.desktop.sendText('A'.repeat(129));assert.equal(calls.length,1);assert.equal(calls[0].params.text.length,129);
});
