import assert from 'node:assert/strict';
import {readFileSync,lstatSync,readdirSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
import path from 'node:path';
import vm from 'node:vm';

const here=path.dirname(fileURLToPath(import.meta.url));
const root=path.resolve(here,'..');
const packageRoot=path.join(root,'examples/actions/n10-rust');
const json=file=>JSON.parse(readFileSync(file,'utf8'));
const manifest=json(path.join(packageRoot,'action.json'));
const call=json(path.join(packageRoot,'call.json'));
const config=json(path.join(root,'examples/n10-rust.json'));
const action=manifest.actions[0];
const entry=path.resolve(packageRoot,action.entry);
const module=await import(pathToFileURL(entry));
const engineSource=readFileSync(path.join(root,'tools/js-engine-probe/src/main.rs'),'utf8');
const rustFields=engineSource.match(/const FAULT_FIELDS: &str = "([\s\S]*?)";/)[1];
const fieldsSource=JSON.parse('"'+rustFields.replace(/\\\r?\n[ \t]*/g,'')+'"');
const projectFault=vm.runInNewContext(fieldsSource,Object.create(null),{timeout:1000});
const reports=[];
function test(name,body){const evidence=body();reports.push({name,result:'passed',...evidence});}
function fields(name){
  const source=readFileSync(path.join(root,'crates/omnitrigger-actions/src/model.rs'),'utf8');
  const block=source.match(new RegExp(`pub struct ${name} \\{([\\s\\S]*?)\\n\\}`))[1];
  return [...block.matchAll(/pub (\w+):/g)].map(row=>row[1]).sort();
}
function fixture(options={}){
  const trace=[];
  const target={window_handle:'9007199254740993',process_id:20,thread_id:30,process_created_file_time:'18446744073709550000',expires_at_unix_ms:Date.now()+20000};
  const ctx={input:{cliExecutable:'R:\\native-fixture\\eaclip-cli.exe',hostExecutable:'R:\\native-fixture\\cliphost-app.exe',
    dataRoot:'R:\\action-data-fixture',profile:'product',timeoutMs:2000,actionParameter:{present:true,value:null}},
    desktop:{targetContext(){trace.push({kind:'targetContext',target:{...target}});return target;}},
    process:{capture(program,args,budget){
      trace.push({kind:'capture',program,args:[...args],budget});
      const result={processId:55,executable:'\\\\?\\r:\\NATIVE-FIXTURE\\cliphost-app.exe',version:'fixture',started:false,
        dataRoot:options.wrongRoot?'R:\\other-root-fixture':'\\\\?\\R:\\action-data-fixture\\',
        targetContextVersion:1,targetContextVerified:true,testProfile:ctx.input.profile==='script-test',captureEnabled:ctx.input.profile!=='script-test'};
      return{code:options.code??0,stdout:JSON.stringify(options.reply??{ok:true,state:'accepted',result}),stderr:''};
    },start(){trace.push({kind:'start'});throw Error('禁止启动');}},
    ui:{setActionHint(options){assert.deepEqual(options,{text:'n10',placement:'topRight'});assert.equal(trace.at(-1).kind,'capture');trace.push({kind:'hint',options});return {updated:true};},notify(){trace.push({kind:'notify'});throw Error('禁止 UI');}}};
  return{ctx,target,trace};
}
function invoke(f){
  let result,error;
  try{result=module.main(f.ctx);}catch(value){error=JSON.parse(projectFault(value));}
  assert.equal(f.trace.filter(row=>row.kind==='start'||row.kind==='notify').length,0);
  return{result,error,trace:f.trace};
}

test('uses exact existing PackageManifest and ActionDefinition fields',()=>{
  assert.deepEqual(Object.keys(manifest).sort(),fields('PackageManifest'));
  assert.deepEqual(Object.keys(action).sort(),fields('ActionDefinition'));
  assert.equal(manifest.schema_version,1);assert.equal(action.runtime,'script');assert.equal(action.single_instance,true);
});
test('identity and display variant preserve original N10 action',()=>{
  assert.equal(manifest.id,'examples.n10-rust');assert.equal(action.id,'32c12786-9bb8-4b0c-8d55-7e6a4c8a5d10');
  assert.equal(action.title,'剪贴板 n10');assert.equal(action.version,1);assert.equal(manifest.actions.length,1);
  for(const value of [manifest.id,action.id])assert(/^[A-Za-z0-9_.-]{1,100}$/.test(value)&&!['.','..'].includes(value));
});
test('entry is local package file and sole frozen implementation',()=>{
  assert.equal(path.dirname(entry),packageRoot);assert.equal(lstatSync(entry).isSymbolicLink(),false);
  assert.equal(typeof module.main,'function');
  assert.equal(createHash('sha256').update(readFileSync(entry)).digest('hex'),'29b2a1f3c8f75d9d9f290bdca1026a89c1ea10cf52859d5c2d2d8b3aae68afb1');
  assert.deepEqual(readdirSync(packageRoot).filter(file=>file.endsWith('.mjs')||file.endsWith('.js')),['n10-rust-bound.mjs']);
});
test('source catalog config resolves to sibling actions and keeps binding disabled',()=>{
  const expectedVersion=Number(readFileSync(path.join(root,'crates/omnitrigger-app/src/settings.rs'),'utf8').match(/const CONFIG_VERSION: u32 = (\d+);/)[1]);
  assert.equal(config.version,expectedVersion);assert.equal(config.bindings.length,1);assert.equal(config.bindings[0].enabled,false);
  assert.equal(path.join(path.dirname(path.join(root,'examples/n10-rust.json')),'actions/n10-rust'),packageRoot);
  const bindingCall={...config.bindings[0].action};assert.equal(bindingCall.kind,'invoke');delete bindingCall.kind;
  assert.deepEqual(bindingCall,call);assert.equal(call.package_id,manifest.id);assert.equal(call.action_id,action.id);assert.equal(call.action_version,action.version);
});
test('unconfigured template has explicit fields and never defaults paths/profile',()=>{
  assert.deepEqual(Object.keys(call.input).sort(),['actionParameter','cliExecutable','dataRoot','hostExecutable','profile','timeoutMs'].sort());
  for(const key of ['cliExecutable','hostExecutable','dataRoot','profile'])assert.equal(call.input[key],null);
  const f=fixture();f.ctx.input=structuredClone(call.input);const reply=invoke(f);
  assert.equal(reply.result,undefined);assert.equal(reply.error.effect_state,'not_started');assert.equal(reply.trace.length,0);
  return reply;
});
test('MIT file copied exactly and notices do not redistribute native components',()=>{
  assert.deepEqual(readFileSync(path.join(packageRoot,'LICENSE')),readFileSync(path.join(root,'LICENSE')));
  const notice=readFileSync(path.join(packageRoot,'THIRD_PARTY_NOTICES.md'),'utf8');
  assert(notice.includes('没有捆绑'));assert(notice.includes('不新增 Cargo/npm 依赖'));
  assert(!readdirSync(packageRoot).some(name=>/\.(exe|dll|node|pdb|zip)$/i.test(name)));
});
test('product files have no machine temp/personal paths or SDK replacement',()=>{
  for(const name of readdirSync(packageRoot)){
    const text=readFileSync(path.join(packageRoot,name),'utf8');
    assert(!/[A-Za-z]:[\\/]Users[\\/]|[A-Za-z]:[\\/]source[\\/]repos|\.tmp[\\/]/i.test(text),name);
  }
  assert(!readFileSync(entry,'utf8').match(/^\s*import\s/m));
});
for(const parameter of [null,'','clip show','  原样参数  '])test(`manifest-resolved main parameter ${String(parameter)}`,()=>{
  const f=fixture();f.ctx.input.actionParameter.value=parameter;const reply=invoke(f);
  assert.equal(reply.error,undefined);assert.equal(reply.result.commandEffectState,'accepted_only');assert.equal(reply.result.uiRendered,false);
  assert.equal(reply.result.retryAllowed,false);assert.deepEqual(reply.trace.map(row=>row.kind),['targetContext','capture','hint']);
  const args=reply.trace[1].args;assert.equal(args[1],parameter===null||parameter===''?'clip toggle':parameter);
  assert(args.includes('--no-start'));assert(args.includes('--expected-data-root'));
  assert.deepEqual(JSON.parse(args[args.indexOf('--target-context')+1]),f.target);return reply;
});
test('script-test is explicit and uses private-root CLI selection',()=>{
  const f=fixture();f.ctx.input.profile='script-test';const reply=invoke(f);assert.equal(reply.error,undefined);
  assert(reply.trace[1].args.includes('--test-data-root'));assert(!reply.trace[1].args.includes('--expected-data-root'));return reply;
});
for(const key of ['cliExecutable','hostExecutable','dataRoot','profile'])test(`missing ${key} rejects before any target/process port`,()=>{
  const f=fixture();delete f.ctx.input[key];const reply=invoke(f);assert.equal(reply.result,undefined);
  assert.equal(reply.error.effect_state,'not_started');assert.equal(reply.trace.length,0);return reply;
});
for(const parameter of [{present:false,value:null},{present:true,value:7},{}])test(`invalid action parameter ${JSON.stringify(parameter)}`,()=>{
  const f=fixture();f.ctx.input.actionParameter=parameter;const reply=invoke(f);assert.equal(reply.result,undefined);
  assert.equal(reply.error.effect_state,'not_started');assert.equal(reply.trace.length,0);return reply;
});
for(const state of ['unknown','partial'])test(`${state} is existing structured error with submitted true and no retry`,()=>{
  const f=fixture({code:5,reply:{ok:false,state,submitted:true,error:{code:'remote_error'}}});const reply=invoke(f);
  assert.equal(reply.result,undefined);assert.equal(reply.error.effect_state,'unknown');assert(reply.error.code.endsWith('_submitted'));
  const detail=JSON.parse(reply.error.message.split('\n').find(line=>line.startsWith('N10_DETAIL=')).slice(11));
  assert.equal(detail.submitted,true);assert.equal(detail.retryAllowed,false);assert.equal(reply.trace.filter(row=>row.kind==='capture').length,1);return reply;
});
test('other data root never becomes acceptance or retry',()=>{
  const f=fixture({wrongRoot:true});const reply=invoke(f);assert.equal(reply.result,undefined);assert.equal(reply.error.effect_state,'unknown');
  assert.equal(reply.trace.filter(row=>row.kind==='capture').length,1);return reply;
});
const result={result:'passed',cases:reports.length,reports,actualEntryFromManifest:path.relative(root,entry).replaceAll('\\','/'),
  actualBusinessModuleSha256:createHash('sha256').update(readFileSync(entry)).digest('hex'),actualNativeProcesses:0,realRustCatalogLoaderExecuted:false};

console.log(JSON.stringify(result));
