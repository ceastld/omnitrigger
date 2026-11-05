// 纯Fake：没有调用真实App、Node/夹具子进程、网络或系统剪贴板。
import assert from 'node:assert/strict';
import test from 'node:test';
import {win32} from 'node:path';
import {
  parseArguments,cases,selectedCases,actionInputForCase,validateOwnedRecords,observeCooperativeCleanup,
  parseAppResult,redactedTerminalSummary,evaluateResult,runAppNodeCapture,
} from '../tools/script-sdk-check/check-app-node-capture.mjs';
import {createCaptureAuthor,AUTHOR_MODULE} from '../tools/script-sdk-check/app-node-capture-author.mjs';

const args=['--app','C:\\Owned\\app.exe','--node','C:\\Owned\\node.exe',
  '--fixture','C:\\Owned\\fixture.exe','--case-root','C:\\Owned\\fresh'];
const config=parseArguments(args);
const token='a'.repeat(32);
const owned={root:'C:\\Owned\\capture-case-test',token};
const record={schemaVersion:1,token,fixture:102,descendant:103,startedAtUnixMs:9900};
const nodeOwner={schemaVersion:1,token,node:101,app:100};
const identities={app:100,node:101,fixtures:[102,103]};
const outputResult=output=>({code:0,stdout:Buffer.from(JSON.stringify({status:'succeeded',output})+'\n'),stderr:Buffer.alloc(0)});
const errorResult=(code,effect,status='failed')=>({code:1,stdout:Buffer.from('{"schema_version":1,"status":"started"}\n'),
  stderr:Buffer.from(JSON.stringify({status,error:{code,...(effect?{effect_state:effect}:{})}})+'\n')});
function authorReceipt(def) {
  if(def.error)return {caseId:def.id,captured:false,errorCode:def.error,effectState:'unknown',methodMatches:true,
    partialOutput:false,messageRedacted:true,nodeMatches:true,returnedBody:false};
  const out=def.expectedOut??'',err=def.expectedErr??'';
  return {caseId:def.id,captured:true,code:def.code,stdoutUnits:out.length,stderrUnits:err.length,
    stdoutBytes:Buffer.byteLength(out),stderrBytes:Buffer.byteLength(err),stdoutMatches:true,stderrMatches:true,
    cwdMatches:true,nodeMatches:true,returnedBody:false};
}
function fakeAdapter(failCase) {
  const launched=[];let released=false,stops=0,saved,now=0;
  return {
    kind:'fake',driverPid:1,now:()=>now,wallNow:()=>10000,sleep:async ms=>{now+=ms;},
    state:id=>id===100?'alive':released?'gone':'alive',
    validate:()=>{},prepare:()=>({root:config.root,config:'synthetic-bindings',packageRoot:'synthetic-package'}),
    warmDirect:()=>({nodeVersion:'v22.0.0',fixtureSignatureVerified:true}),
    createCase:(_config,_layout,def)=>{released=false;return {...owned,id:def.id};},
    launch:async(_config,_layout,own)=>{
      const def=cases().find(c=>c.id===own.id);launched.push(def.id);
      let closed=false;
      const result=def.id===failCase?outputResult({caseId:def.id,stdout:'PRIVATE_MUST_NOT_LEAK'}):
        def.control?errorResult(def.appError,def.id==='node-self-exit'?undefined:'unknown',def.appStatus):outputResult(authorReceipt(def));
      return {pid:100,get closed(){return closed;},get done(){closed=true;return Promise.resolve(result);},stop(){stops++;closed=true;}};
    },
    controlRecords:()=>({record:{...record},nodeOwner:{...nodeOwner}}),
    release:()=>{released=true;},verifyControlMarker:()=>{},save:(_layout,receipt)=>{saved=JSON.parse(JSON.stringify(receipt));},
    get launched(){return launched;},get stops(){return stops;},get saved(){return saved;},
  };
}

test('路径必须全部显式本地绝对EXE，拒绝重复/未知参数和过大超时',()=>{
  assert.equal(config.coldTimeoutMs,60000);assert.equal(config.caseTimeoutMs,20000);assert.equal(config.observeMs,3000);
  for(const bad of [args.slice(0,-2),[...args,'--app','C:\\Owned\\other.exe'],[...args,'--unknown','x'],
    [...args,'--observe-ms','15000'],args.map(x=>x==='C:\\Owned\\node.exe'?'node.exe':x),
    args.map(x=>x==='C:\\Owned\\fixture.exe'?'\\\\server\\fixture.exe':x),
    args.map(x=>x==='C:\\Owned\\node.exe'?'C:\\Owned\\..\\node.exe':x)]) {
    assert.throws(()=>parseArguments(bad));
  }
});
test('64KiB双流期望不放进ActionCall输入，避免被64KiB调用上限先拒绝',()=>{
  for(const def of cases()) {
    const input=actionInputForCase(def,config,owned,def.options??{},undefined);
    assert(Buffer.byteLength(JSON.stringify(input))<2048);
  }
  const exact=actionInputForCase(cases().find(c=>c.id==='dual-exact'),config,owned,{},undefined);
  assert.equal(exact.expectedOut,undefined);assert.equal(exact.expectedOutUnits,32768);
});
test('PID只接受新鲜同token的本次App/Node/夹具记录，碰撞和自然退出后证据拒绝',()=>{
  assert.deepEqual(validateOwnedRecords(record,nodeOwner,owned,100,1,10000),identities);
  for(const [r,n,wall] of [[{...record,token:'wrong'},nodeOwner,10000],[record,{...nodeOwner,node:100},10000],
    [record,{...nodeOwner,app:999},10000],[{...record,fixture:1},nodeOwner,10000],[record,nodeOwner,30000]]) {
    assert.throws(()=>validateOwnedRecords(r,n,owned,100,1,wall));
  }
});
test('只在Node和后代已消失且App前后存活时记录协作清理证据',async()=>{
  let now=0;const observed=[];
  const result=await observeCooperativeCleanup(identities,{
    now:()=>now,sleep:async ms=>{now+=ms;},appClosed:()=>false,
    state:id=>{observed.push(id);return id===100?'alive':now>=4?'gone':'alive';},
  },100);
  assert.equal(result.elapsedMs,4);assert.equal(result.appAliveBeforeAndAfter,true);
  assert.equal(result.naturalExpiryUsed,false);assert(observed.every(id=>[100,101,102,103].includes(id)));
});
test('App先退出不能拿OS关Job冒充协作取消，未知观察和超时不重试',async()=>{
  for(const mode of ['app-exit','unknown','timeout']) {
    let now=0;
    await assert.rejects(observeCooperativeCleanup(identities,{
      now:()=>now,sleep:async ms=>{now+=ms;},appClosed:()=>mode==='app-exit',
      state:id=>mode==='unknown'?'unknown':mode==='app-exit'&&id===100?'gone':'alive',
    },100));
    assert(now<=100);
  }
});
test('App混合stdout/stderr只接受唯一终态，坏UTF8与重复回执不接受',()=>{
  assert.equal(parseAppResult(errorResult('runtime_exit')).error.code,'runtime_exit');
  assert.throws(()=>parseAppResult({code:0,stdout:Buffer.from([0xff]),stderr:Buffer.alloc(0)}));
  assert.throws(()=>parseAppResult({code:0,stdout:Buffer.from('{"status":"succeeded"}\n{"status":"succeeded"}\n'),stderr:Buffer.alloc(0)}));
  assert.throws(()=>parseAppResult({...outputResult({}),timedOut:true}));
});
test('终态只保留长度/代码/布尔值，原始stdout字段及不正确错误语义明确失败',()=>{
  const def=cases()[0],valid=authorReceipt(def);
  assert.equal(evaluateResult(def,outputResult(valid)).passed,true);
  assert.throws(()=>evaluateResult(def,outputResult({...valid,stdout:'PRIVATE_MUST_NOT_LEAK'})));
  const failure=cases().find(c=>c.id==='invalid-utf8');
  assert.throws(()=>evaluateResult(failure,outputResult({...authorReceipt(failure),effectState:'not_started'})));
});
test('12项完整Fake流程含冷启动预热和3种控制边界，无真实运行或重试',async()=>{
  const adapter=fakeAdapter();const receipt=await runAppNodeCapture(config,adapter);
  assert.equal(receipt.passed,true);assert.equal(receipt.execution,'fake');assert.equal(receipt.cases.length,12);
  assert.equal(new Set(adapter.launched).size,12);assert.equal(adapter.stops,0);
  assert.equal(receipt.unknownRetried,false);assert.equal(receipt.outerActionDeadlineVerified,false);
  assert.equal(receipt.actionServiceUserCancelVerified,false);assert.deepEqual(adapter.saved,receipt);
  assert(!JSON.stringify(receipt).includes('controlled output'));assert(!JSON.stringify(receipt).includes('stdout":'));
});
test('出现未知错误立即停止该批，不泄露正文、不重启同一动作',async()=>{
  const adapter=fakeAdapter('invalid-utf8');const receipt=await runAppNodeCapture(config,adapter);
  assert.equal(receipt.passed,false);assert.equal(receipt.cases.length,4);
  assert.equal(adapter.launched.filter(id=>id==='invalid-utf8').length,1);
  assert(!JSON.stringify(receipt).includes('PRIVATE_MUST_NOT_LEAK'));assert.equal(receipt.unknownRetried,false);
});
test('协作观测失败时只停止本次返回的App句柄，不按记录PID终止且不继续动作',async()=>{
  const adapter=fakeAdapter();let released=false;
  adapter.release=()=>{released=true;};adapter.state=()=>released?'gone':'alive';
  const receipt=await runAppNodeCapture(config,adapter);
  assert.equal(receipt.passed,false);assert.equal(receipt.error.code,'app_exited_before_cleanup_observation');
  assert.equal(adapter.stops,1);assert.equal(adapter.launched.at(-1),'node-cancel-frame');
  assert(!adapter.launched.includes('node-self-exit'));assert.equal(receipt.unknownRetried,false);
});
test('生成的作者模块可编译，且没有执行任何导入或入口',()=>{
  const body=AUTHOR_MODULE.split('\n').filter(line=>!line.startsWith('import ')).join('\n').replace('export default ','return ');
  assert.doesNotThrow(()=>new Function('existsSync','readFileSync','writeFileSync','join',body));
});

function authorHarness(caseId) {
  const files=new Map(),kills=[];let now=0,cancelled=false,rejectPending;
  const pending=new Promise((_,reject)=>{rejectPending=reject;});
  const root=owned.root;
  files.set(win32.join(root,'app-owner.json'),JSON.stringify({token,app:100}));
  files.set(win32.join(root,'fixture-ready'),token);files.set(win32.join(root,'release-control'),token);
  const platform={proc:{pid:101,ppid:100,execPath:config.node,kill:(id,signal)=>{
    kills.push({id,signal});throw Object.assign(Error('fake-self-exit'),{code:'synthetic_self_exit'});
  }},join:win32.join,now:()=>now,sleep:async ms=>{now+=ms;},exists:path=>files.has(path),
  read:path=>files.get(path),writeNew:(path,text)=>{assert(!files.has(path));files.set(path,text);},
  byteLength:Buffer.byteLength,cancelHost:()=>{cancelled=true;rejectPending(Object.assign(Error('cancelled'),{code:'cancelled'}));}};
  const ctx={input:{caseId,expectedNode:config.node,root,token,program:config.fixture},process:{capture:()=>pending}};
  return {run:()=>createCaptureAuthor(platform)(ctx),kills,files,get cancelled(){return cancelled;}};
}
test('合成作者强杀只针对自身PID，不从夹具记录读取任何kill目标',async()=>{
  const h=authorHarness('node-self-exit');
  await assert.rejects(h.run(),error=>error.code==='synthetic_self_exit');
  assert.deepEqual(h.kills,[{id:101,signal:'SIGKILL'}]);
  assert.equal(h.files.get(win32.join(owned.root,'node-control-request')),token);
  assert(AUTHOR_MODULE.includes('proc.kill(proc.pid'));
});
test('合成取消帧与未await分支分开记录，不伪称用户取消或退出同一OS保障',async()=>{
  const cancel=authorHarness('node-cancel-frame');
  await assert.rejects(cancel.run(),error=>error.code==='cancelled');assert.equal(cancel.cancelled,true);assert.deepEqual(cancel.kills,[]);
  const ignored=authorHarness('unawaited');
  assert.deepEqual(await ignored.run(),{caseId:'unawaited',returnedWithoutAwait:true});assert.deepEqual(ignored.kills,[]);
});


test('取消用例严格遵循ActionService到CLI的cancelled终态，不能以failed同码蒙混',()=>{
  const def=cases().find(c=>c.id==='node-cancel-frame');
  const observation={nodeGone:true,fixturesGone:true,appAliveBeforeAndAfter:true,elapsedMs:10,naturalExpiryUsed:false};
  assert.equal(evaluateResult(def,errorResult('cancelled','unknown','cancelled'),observation).passed,true);
  assert.throws(()=>evaluateResult(def,errorResult('cancelled','unknown','failed'),observation));
  assert.throws(()=>evaluateResult(def,errorResult('cancelled','not_started','cancelled'),observation));
  assert.throws(()=>evaluateResult(def,errorResult('cancelled','unknown','cancelled'),{...observation,appAliveBeforeAndAfter:false}));
  const own=cases().find(c=>c.id==='unawaited');
  assert.throws(()=>evaluateResult(own,errorResult('pending_process_capture','unknown','cancelled'),observation));
});
test('脱敏终态摘要严格只有status/code/effect/method，不携带message或output正文',()=>{
  const summary=redactedTerminalSummary({status:'cancelled',error:{code:'cancelled',effect_state:'unknown',method:'process.capture',message:'PRIVATE_BODY'},output:{stdout:'PRIVATE_BODY'}});
  assert.deepEqual(summary,{status:'cancelled',code:'cancelled',effect:'unknown',method:'process.capture'});
  assert.deepEqual(Object.keys(summary).sort(),['code','effect','method','status']);
  assert(!JSON.stringify(summary).includes('PRIVATE_BODY'));
  assert.deepEqual(redactedTerminalSummary({status:'PRIVATE_BODY',error:{code:'PRIVATE_BODY',effect_state:'PRIVATE_BODY',method:'PRIVATE_BODY'}}),{status:null,code:null,effect:null,method:null});
});
test('真实可解析终态在验收断言失败前记录，原始正文不会进入失败receipt',async()=>{
  const adapter=fakeAdapter('invalid-utf8');
  const receipt=await runAppNodeCapture(config,adapter);
  assert.equal(receipt.passed,false);
  assert.deepEqual(receipt.cases.at(-1).terminal,{status:'succeeded',code:null,effect:null,method:null});
  assert(!JSON.stringify(receipt).includes('PRIVATE_MUST_NOT_LEAK'));
});

test('App先退出而清理未证明时可记录已有终态，不能把该终态变成通过',async()=>{
  const adapter=fakeAdapter();let released=false;
  adapter.release=()=>{released=true;};adapter.state=()=>released?'gone':'alive';
  const originalLaunch=adapter.launch;
  adapter.launch=async(...args)=>{
    const child=await originalLaunch(...args);
    if(args[2].id!=='node-cancel-frame')return child;
    return {pid:child.pid,get closed(){return released||child.closed;},
      get done(){return Promise.resolve(errorResult('runtime_exit'));},stop:child.stop};
  };
  const receipt=await runAppNodeCapture(config,adapter);
  assert.equal(receipt.passed,false);
  assert.equal(receipt.error.code,'app_exited_before_cleanup_observation');
  assert.deepEqual(receipt.cases.at(-1).terminal,{status:'failed',code:'runtime_exit',effect:null,method:null});
  assert.equal(receipt.cases.at(-1).passed,false);assert.equal(receipt.unknownRetried,false);
});

test('默认all保留12项，normal仅9项，control仅3项且没有隐式fallback',async()=>{
  assert.equal(parseArguments(args).suite,'all');assert.equal(selectedCases('all').length,12);
  assert.equal(selectedCases('normal').length,9);assert.equal(selectedCases('control').length,3);
  assert(selectedCases('normal').every(def=>!def.control));assert(selectedCases('control').every(def=>def.control));
  for(const suite of ['all','normal','control']) {
    const adapter=fakeAdapter();const receipt=await runAppNodeCapture({...config,suite},adapter);
    assert.equal(receipt.suite,suite);assert.equal(receipt.plannedCases,selectedCases(suite).length);
    assert.deepEqual(adapter.launched,selectedCases(suite).map(def=>def.id));
  }
  assert.throws(()=>parseArguments([...args,'--suite','auto']));
  assert.throws(()=>selectedCases('normal-or-control'));
});
test('control失败维持失败，绝不自动转normal9或继续重试',async()=>{
  const adapter=fakeAdapter('node-cancel-frame');
  const receipt=await runAppNodeCapture({...config,suite:'control'},adapter);
  assert.equal(receipt.passed,false);assert.equal(receipt.suite,'control');assert.equal(receipt.plannedCases,3);
  assert.deepEqual(adapter.launched,['node-cancel-frame']);assert.equal(receipt.cases.length,1);
  assert.equal(receipt.unknownRetried,false);
});
