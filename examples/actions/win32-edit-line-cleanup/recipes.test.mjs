import assert from 'node:assert/strict';
import {test} from 'node:test';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import nodeMain from './node.mjs';
import {main as scriptMain} from './script.mjs';
const sourceRoot=process.env.OMNITRIGGER_CONTRACT_SOURCE_ROOT??fileURLToPath(new URL('../../../',import.meta.url));
const desktop=readFileSync(join(sourceRoot,'crates/omnitrigger-actions/runtime/desktop-api.js'),'utf8');
const host=readFileSync(join(sourceRoot,'crates/omnitrigger-actions/runtime/host.mjs'),'utf8');
const sdk=readFileSync(join(sourceRoot,'tools/js-engine-probe/src/sdk.js'),'utf8');
const asyncApi=host.split('\n').find(line=>line.startsWith('const asyncApi ='));
const method='win32_edit.replace_selection',receipt={messageProcessed:true,effectState:'unknown'};
const json=x=>JSON.parse(JSON.stringify(x));
function harness(runtime,{text='A\r\n\r\nB😀',second=text,input={adapter:'winforms-edit'},write=receipt,writeError,onRead=()=>{}}={}){
  const calls=[];let reads=0;
  const signal={aborted:false};
  function invoke(method,params){calls.push({method,params:json(params)});if(method==='selection.read_text'){reads++;onRead(reads,signal);return {text:reads===1?text:second,truncated:false};}if(method!=='win32_edit.replace_selection')throw Error('unexpected fallback');if(writeError)throw writeError;return write;}
  if(runtime==='node'){
    const context=vm.createContext({invoke});vm.runInContext(desktop+'\n'+asyncApi+'\nglobalThis.api=asyncApi(createDesktopApi(invoke).desktop);',context);
    const ctx={input,signal,desktop:context.api};return {calls,signal,run:()=>nodeMain(ctx)};
  }
  const context=vm.createContext({ctx:{input,__nativeContext:{connection:{port:9,token:'a'.repeat(64)}}},
    __hostCall(raw){const request=JSON.parse(raw);try{return JSON.stringify({ok:true,value:invoke(request.method,request.params)});}catch(error){return JSON.stringify({ok:false,error:{code:error.code,message:'private',effect_state:error.effectState,method:error.method}});}},
    createProcessCaptureApi:()=>({capture(){}}),createBrowserApi:()=>({}),createClipboardHistoryApi:()=>({})});
  vm.runInContext(desktop+'\n'+sdk,context);return {calls,signal,run:()=>scriptMain(context.ctx)};
}
async function fails(e,code,effect='not_started',expectedMethod){await assert.rejects(async()=>e.run(),x=>x.code===code&&x.effectState===effect&&(!expectedMethod||x.method===expectedMethod));}
function readOptions(e,count){assert.deepEqual(e.calls.filter(x=>x.method==='selection.read_text').map(x=>x.params),Array.from({length:count},()=>({overflow:'reject',max_utf16_units:8192})));}
for(const runtime of ['node','script']){
  for(const [operation,expected] of [['remove-empty-lines','A\r\n \t\r\nB😀'],['remove-white-space-lines','A\r\nB😀']])test(`${runtime} ${operation}复用行语义且2读1写`,async()=>{
    const e=harness(runtime,{text:'A\r\n\r\n \t\r\nB😀',input:{operation,adapter:'winforms-edit'}});
    assert.deepEqual(json(await e.run()),{status:'replacement_message_processed',effectState:'unknown'});readOptions(e,2);
    assert.deepEqual(e.calls.map(x=>x.method),['selection.read_text','selection.read_text',method]);assert.deepEqual(e.calls[2].params,{text:expected,adapter:'winforms-edit'});
  });
  test(`${runtime} 显式native-edit适配原样传递`,async()=>{const e=harness(runtime,{input:{adapter:'native-edit'}});await e.run();assert.equal(e.calls[2].params.adapter,'native-edit');});
  for(const [name,text,status] of [['空选区','','no_selection'],['未变化','A\r\nB😀','unchanged'],['8192未变化','😀'.repeat(4096),'unchanged']])test(`${runtime} ${name}仅一读零写`,async()=>{const e=harness(runtime,{text});assert.equal((await e.run()).status,status);assert.equal(e.calls.length,1);});
  test(`${runtime} 非空全空行/空白选区二读一致后一次空替换`,async()=>{
    for(const [operation,text] of [['remove-empty-lines','\r\n\n\r\n'],['remove-white-space-lines',' \t\r\n\u0085\r\n\u3000']]){
      const e=harness(runtime,{text,input:{operation,adapter:'winforms-edit'}});
      assert.deepEqual(json(await e.run()),{status:'replacement_message_processed',effectState:'unknown'});
      readOptions(e,2);assert.deepEqual(e.calls.map(x=>x.method),['selection.read_text','selection.read_text',method]);
      assert.deepEqual(e.calls[2].params,{text:'',adapter:'winforms-edit'});
      const changed=harness(runtime,{text,second:'CHANGED',input:{operation,adapter:'winforms-edit'}});
      await fails(changed,'selection_changed');assert.equal(changed.calls.length,2);
      const unknown=harness(runtime,{text,input:{operation,adapter:'winforms-edit'},writeError:Object.assign(new Error('private'),{code:'timeout',method,effectState:'unknown'})});
      await fails(unknown,'timeout','unknown',method);assert.equal(unknown.calls.length,3);assert.equal(unknown.calls[2].params.text,'');
    }
  });
  test(`${runtime} .NET空白与FEFF/200B保持既有区分`,async()=>{
    const e=harness(runtime,{text:'A\r\n\u0085\r\n\u200b\r\n\ufeff\r\nB',input:{adapter:'winforms-edit',operation:'remove-white-space-lines'}});await e.run();assert.equal(e.calls[2].params.text,'A\r\n\u200b\r\n\ufeff\r\nB');
  });
  test(`${runtime} 8192源缩为短结果`,async()=>{const e=harness(runtime,{text:'\n'.repeat(8184)+'A\r\n\r\nB😀'});await e.run();assert.equal(e.calls[2].params.text,'A\r\nB😀');});
  test(`${runtime} 输出8192完整提交不截断`,async()=>{const e=harness(runtime,{text:'A'.repeat(8189)+'\nB'});await e.run();assert.equal(e.calls[2].params.text.length,8192);});
  test(`${runtime} 输出8193在第二读之前拒绝`,async()=>{const e=harness(runtime,{text:'A'.repeat(8190)+'\nB'});await fails(e,'replacement_too_long','not_started',method);assert.equal(e.calls.length,1);});
  test(`${runtime} 8193源由实际SDK拒绝且零写`,async()=>{const e=harness(runtime,{text:'A'.repeat(8193)});await fails(e,'invalid_response');assert.equal(e.calls.length,1);});
  test(`${runtime} 第二读变化拒绝零写`,async()=>{const e=harness(runtime,{second:'CHANGED'});await fails(e,'selection_changed');assert.equal(e.calls.length,2);});
  for(const input of [undefined,{},null,{adapter:'auto'},{adapter:'winforms-edit',hwnd:10},{adapter:'native-edit',operation:'other'}])test(`${runtime} 输入${JSON.stringify(input)}拒绝且零调用`,async()=>{
    const e=harness(runtime,{input:input===undefined?{adapter:undefined}:input});await fails(e,'invalid_input');assert.equal(e.calls.length,0);
  });
  for(const [code,effect] of [['timeout','unknown'],['edit_message_unconfirmed','unknown'],['target_changed','not_started'],['edit_unsupported','not_started'],['cancelled','not_started']])test(`${runtime} ${code}/${effect}一次提交无fallback`,async()=>{
    const e=harness(runtime,{writeError:Object.assign(new Error('private original'),{code,effectState:effect,method})});await fails(e,code,effect,method);assert.deepEqual(e.calls.map(x=>x.method),['selection.read_text','selection.read_text',method]);
  });
  for(const write of [{messageProcessed:false,effectState:'unknown'},{messageProcessed:true,effectState:'not_started'},{...receipt,text:'private'}])test(`${runtime} 坏回执unknown且不重试`,async()=>{const e=harness(runtime,{write});await fails(e,'invalid_response','unknown',method);assert.equal(e.calls.length,3);});
}
for(const at of [1,2])test(`Node第${at}读后取消不提交`,async()=>{const e=harness('node',{onRead(read,signal){if(read===at)signal.aborted=true;}});await fails(e,'cancelled');assert.equal(e.calls.length,at);});
