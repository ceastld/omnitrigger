// 只运行真实入口与共享SDK的录制端口；不启动clip、不访问剪贴板或真实UI。
import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
import {resolve,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {main,runBusiness} from '../examples/actions/n10-rust/n10-rust-bound.mjs';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const source=name=>readFileSync(resolve(root,'crates/omnitrigger-actions/runtime',name),'utf8');
const factories=vm.runInNewContext(source('desktop-api.js')+'\n'+source('process-capture-api.js')+'\n({createDesktopApi,createProcessCaptureApi})');
const plain=value=>JSON.parse(JSON.stringify(value));
const input={cliExecutable:'C:\\Synthetic\\eaclip-cli.exe',hostExecutable:'C:\\Synthetic\\cliphost.exe',dataRoot:'C:\\Synthetic\\data',profile:'script-test',timeoutMs:1000,actionParameter:{present:true,value:null}};
function setup({clip='accepted',hint={updated:true},hintError,missingHint=false}={}) {
  const target={window_handle:'101',process_created_file_time:'134000000000000001',expires_at_unix_ms:Date.now()+20000};
  const calls=[];
  function host(method,raw){
    const params=plain(raw);calls.push({method,params});
    if(method==='target.context'){assert.equal(calls.length,1);assert.deepEqual(params,{});return target;}
    if(method==='process.capture'){
      assert.equal(calls.length,2);
      assert.deepEqual(params,{program:input.cliExecutable,args:['verb','clip toggle','--json','--host',input.hostExecutable,'--no-start','--test-data-root',input.dataRoot,'--target-context',JSON.stringify(target),'--timeout-ms','1000'],timeout_ms:2000,allow_non_zero:true});
      if(clip==='throw')throw Object.assign(Error('secret capture error'),{code:'process_timeout',effectState:'unknown'});
      if(clip==='partial')return {code:1,stdout:JSON.stringify({ok:false,state:'partial',submitted:true,result:{status:'partial',commandEffectState:'partial'}}),stderr:''};
      if(clip==='unknown')return {code:1,stdout:JSON.stringify({ok:false,state:'unknown',submitted:true,error:{code:'synthetic_failure'}}),stderr:''};
      if(clip==='rejected')return {code:1,stdout:JSON.stringify({ok:false,state:'failed',submitted:false,error:{code:'synthetic_failure'}}),stderr:''};
      return {code:0,stdout:JSON.stringify({ok:true,state:'accepted',submitted:true,result:{started:false,targetContextVersion:1,targetContextVerified:true,executable:input.hostExecutable,dataRoot:input.dataRoot,testProfile:true,captureEnabled:false}}),stderr:''};
    }
    assert.equal(method,'ui.action_hint','不允许启动程序、重新选目标、剪贴板或其他Host调用');
    assert.equal(calls.length,3);assert.deepEqual(params,{text:'n10',placement:'topRight'});
    if(hintError)throw Object.assign(Error('secret host hint body'),hintError);
    return hint;
  }
  const api=factories.createDesktopApi(host);
  const ctx={input:structuredClone(input),desktop:api.desktop,process:factories.createProcessCaptureApi(params=>host('process.capture',params)),ui:missingHint?{}:api.ui};
  return {ctx,calls};
}
function detail(error){
  const line=error.message.split('\n').find(x=>x.startsWith('N10_DETAIL='));assert(line);
  assert(!error.message.includes('secret'));assert(!error.message.includes(input.dataRoot));
  return JSON.parse(line.slice('N10_DETAIL='.length));
}
test('N10正常入口：clip接纳后恰好一次右上角角标，保留原accepted_only输出',()=>{
  const {ctx,calls}=setup();const result=main(ctx);
  assert.deepEqual(calls.map(x=>x.method),['target.context','process.capture','ui.action_hint']);
  assert.equal(result.status,'accepted');assert.equal(result.commandEffectState,'accepted_only');assert.equal(result.uiRendered,false);assert.equal(result.retryAllowed,false);
  assert(!Object.hasOwn(result,'hintVisible'));assert(!Object.hasOwn(result,'badgeRendered'));
});
for(const clip of ['unknown','partial','rejected','throw'])test(`N10 ${clip}业务错误保持原错误链且不写角标`,()=>{
  const {ctx,calls}=setup({clip});
  assert.throws(()=>main(ctx),error=>{const d=detail(error);assert.equal(d.phase,'business');assert.equal(error.method,'cliphost.runVerbWithTargetV1');assert.equal(d.retryAllowed,false);return true;});
  assert.deepEqual(calls.map(x=>x.method),['target.context','process.capture']);
});
for(const hint of [null,{updated:false},{updated:true,extra:'secret'}])test(`N10角标坏回执 ${JSON.stringify(hint)}：保留业务接纳与整轮unknown`,()=>{
  const {ctx,calls}=setup({hint});
  assert.throws(()=>main(ctx),error=>{
    assert.equal(error.code,'n10_partial_submitted');assert.equal(error.method,'ui.action_hint');assert.equal(error.effectState,'unknown');
    const d=detail(error);assert.equal(d.phase,'action_hint');assert.equal(d.submitted,true);assert.equal(d.businessAccepted,true);assert.equal(d.businessEffectState,'accepted_only');assert.equal(d.hintEffectState,'unknown');assert.equal(d.retryAllowed,false);return true;
  });assert.equal(calls.length,3);
});
for(const effectState of ['not_started','unknown'])test(`N10角标 ${effectState} 拒绝不能把已接纳clip降为未开始`,()=>{
  const {ctx,calls}=setup({hintError:{code:'ui_unavailable',method:'ui.action_hint',effectState}});
  assert.throws(()=>main(ctx),error=>{assert.equal(error.effectState,'unknown');const d=detail(error);assert.equal(d.businessAccepted,true);assert.equal(d.hintEffectState,effectState);assert.equal(d.submitted,true);return true;});assert.equal(calls.length,3);
});
test('N10缺角标API：clip接纳后明确部分失败，不重发clip',()=>{
  const {ctx,calls}=setup({missingHint:true});
  assert.throws(()=>main(ctx),error=>{const d=detail(error);assert.equal(d.businessAccepted,true);assert.equal(d.hintEffectState,'not_started');assert.equal(error.effectState,'unknown');return true;});assert.equal(calls.length,2);
});
test('N10坏输入在任何录制Host调用前拒绝',()=>{
  const {ctx,calls}=setup();ctx.input.actionParameter={present:false,value:null};
  assert.throws(()=>main(ctx),error=>{assert.equal(error.effectState,'not_started');assert.equal(detail(error).phase,'business');return true;});assert.equal(calls.length,0);
});

test('业务复用入口保留原业务体，不替文件菜单写后置角标',()=>{
  const {ctx,calls}=setup({missingHint:true});const result=runBusiness(ctx);
  assert.equal(result.status,'accepted');assert.equal(result.commandEffectState,'accepted_only');
  assert.deepEqual(calls.map(x=>x.method),['target.context','process.capture']);
});
