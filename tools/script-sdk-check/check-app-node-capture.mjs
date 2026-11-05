// 显式App --run-action验收；导入不启动进程。真实执行只使用用户给定的自有EXE。
import {spawn,spawnSync} from 'node:child_process';
import {randomBytes} from 'node:crypto';
import {existsSync,lstatSync,mkdirSync,mkdtempSync,openSync,closeSync,fstatSync,readSync,realpathSync,renameSync,writeFileSync} from 'node:fs';
import {win32,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {verifyOrdinaryPaths} from './check-clipboard-history.mjs';
import {AUTHOR_MODULE} from './app-node-capture-author.mjs';

export const fixtureSignature='omnitrigger-process-capture-fixture-v1';
class CheckFailure extends Error {
  constructor(code){super('App Node采集验收未完成；请检查回执代码。');this.code=code;}
}
const requireCheck=(condition,code)=>{if(!condition)throw new CheckFailure(code);};
const local=path=>typeof path==='string'&&path===path.trim()&&/^[A-Za-z]:[\\/]/.test(path)
  &&!path.includes('\0')&&!path.slice(3).split(/[\\/]/).some(part=>part==='.'||part==='..');
const pid=value=>Number.isInteger(value)&&value>0&&value<=0xffffffff;
const pause=ms=>new Promise(done=>setTimeout(done,ms));

export function parseArguments(args) {
  const names={'--app':'app','--node':'node','--fixture':'fixture','--case-root':'root',
    '--suite':'suite','--cold-timeout-ms':'coldTimeoutMs','--case-timeout-ms':'caseTimeoutMs','--observe-ms':'observeMs'};
  const result={suite:'all',coldTimeoutMs:60000,caseTimeoutMs:20000,observeMs:3000};
  const seen=new Set();
  for(let i=0;i<args.length;i+=2) {
    const name=names[args[i]];
    requireCheck(name&&!seen.has(name)&&typeof args[i+1]==='string','invalid_arguments');
    seen.add(name);result[name]=name.endsWith('Ms')?Number(args[i+1]):args[i+1];
  }
  requireCheck(['all','normal','control'].includes(result.suite),'invalid_suite');
  for(const name of ['app','node','fixture','root'])requireCheck(local(result[name]),'explicit_local_paths_required');
  for(const name of ['app','node','fixture'])requireCheck(result[name].toLowerCase().endsWith('.exe'),'explicit_exe_required');
  requireCheck(new Set(['app','node','fixture'].map(name=>win32.normalize(result[name]).toLowerCase())).size===3,'executables_must_differ');
  requireCheck(win32.dirname(result.root)!==result.root,'fresh_case_root_required');
  for(const [name,min,max] of [['coldTimeoutMs',1000,120000],['caseTimeoutMs',1000,90000],['observeMs',100,5000]]) {
    requireCheck(Number.isInteger(result[name])&&result[name]>=min&&result[name]<=max,'invalid_timeout');
  }
  return result;
}

export function cases() {
  return [
    {id:'warmup',args:['warmup'],expectedOut:fixtureSignature,expectedErr:'',code:0,cold:true},
    {id:'dual-exact',args:['dual','32768','32768'],expectedOut:'a'.repeat(32768),expectedErr:'b'.repeat(32768),code:0},
    {id:'dual-overflow',args:['dual','32769','32768'],error:'process_output_too_large'},
    {id:'invalid-utf8',args:['invalid-utf8'],error:'process_invalid_utf8'},
    {id:'nonzero-default',args:['exit','7'],error:'process_exit_nonzero'},
    {id:'nonzero-allowed',args:['exit','7'],options:{allowNonZero:true},expectedOut:'controlled output',expectedErr:'',code:7},
    {id:'cwd-default',args:['cwd'],cwd:'package',code:0},
    {id:'cwd-explicit',args:['cwd'],cwd:'explicit',code:0},
    {id:'capture-timeout',args:['sleep','1000'],options:{timeoutMs:10},error:'process_timeout'},
    {id:'node-cancel-frame',control:true,appError:'cancelled',appStatus:'cancelled'},
    {id:'unawaited',control:true,appError:'pending_process_capture',appStatus:'failed'},
    {id:'node-self-exit',control:true,appError:'runtime_exit',appStatus:'failed'},
  ];
}

export function selectedCases(suite) {
  requireCheck(['all','normal','control'].includes(suite),'invalid_suite');
  const available=cases();
  return suite==='all'?available:available.filter(def=>suite==='control'?def.control:!def.control);
}

export function actionInputForCase(def,config,owned,options,expectedCwd) {
  const input={caseId:def.id,program:config.fixture,args:def.args??[],options,expectedNode:config.node,
    root:owned.root,token:owned.token,expectedOut:def.expectedOut,expectedErr:def.expectedErr,expectedCwd};
  if(def.id==='dual-exact') {
    delete input.expectedOut;delete input.expectedErr;
    Object.assign(input,{expectedOutChar:'a',expectedOutUnits:32768,expectedErrChar:'b',expectedErrUnits:32768});
  }
  return input;
}

export function validateOwnedRecords(record,nodeOwner,owned,appPid,driverPid,wallNow) {
  requireCheck(record?.schemaVersion===1&&nodeOwner?.schemaVersion===1
    &&record.token===owned.token&&nodeOwner.token===owned.token,'owner_record_mismatch');
  requireCheck(nodeOwner.app===appPid&&pid(appPid)&&pid(nodeOwner.node)
    &&pid(record.fixture)&&pid(record.descendant),'owner_pid_invalid');
  const values=[appPid,nodeOwner.node,record.fixture,record.descendant,driverPid];
  requireCheck(values.every(pid)&&new Set(values).size===values.length,'owner_pid_collision');
  const age=wallNow-record.startedAtUnixMs;
  requireCheck(Number.isSafeInteger(record.startedAtUnixMs)&&age>=0&&age<5000,'fixture_age_unproven');
  return {app:appPid,node:nodeOwner.node,fixtures:[record.fixture,record.descendant]};
}

export async function observeCooperativeCleanup(identities,ops,timeoutMs) {
  const start=ops.now();
  while(ops.now()-start<timeoutMs) {
    const before=ops.state(identities.app);
    const node=ops.state(identities.node);
    const children=identities.fixtures.map(value=>ops.state(value));
    const after=ops.state(identities.app);
    requireCheck(![before,node,...children,after].includes('unknown'),'pid_observation_unknown');
    requireCheck(before==='alive'&&after==='alive'&&!ops.appClosed(),'app_exited_before_cleanup_observation');
    if(node==='gone'&&children.every(value=>value==='gone')) {
      return {nodeGone:true,fixturesGone:true,appAliveBeforeAndAfter:true,
        elapsedMs:Math.round(ops.now()-start),naturalExpiryUsed:false};
    }
    await ops.sleep(2);
  }
  throw new CheckFailure('cooperative_cleanup_not_observed');
}

export function parseAppResult(result) {
  requireCheck(!result.timedOut&&!result.outputLimited&&!result.spawnFailed,'app_process_incomplete');
  requireCheck(Number.isInteger(result.code),'app_exit_unknown');
  let messages;
  try {
    const decoder=()=>new TextDecoder('utf-8',{fatal:true});
    messages=[decoder().decode(result.stdout),decoder().decode(result.stderr)].flatMap(text=>
      text.split(/\r?\n/).filter(line=>line.trim()).map(line=>JSON.parse(line)));
  } catch {throw new CheckFailure('app_receipt_invalid');}
  const final=messages.filter(item=>['succeeded','failed','rejected','cancelled'].includes(item?.status));
  requireCheck(final.length===1&&messages.every(item=>item.status==='started'||final.includes(item)),'app_receipt_ambiguous');
  return final[0];
}
export function redactedTerminalSummary(final) {
  return {
    status:['succeeded','failed','rejected','cancelled'].includes(final?.status)?final.status:null,
    code:typeof final?.error?.code==='string'&&/^[a-z][a-z0-9_]{0,63}$/.test(final.error.code)?final.error.code:null,
    effect:['not_started','unknown'].includes(final?.error?.effect_state)?final.error.effect_state:null,
    method:typeof final?.error?.method==='string'&&/^[a-z][a-z0-9_.]{0,95}$/.test(final.error.method)?final.error.method:null,
  };
}
export function evaluateResult(def,result,observation) {
  const final=parseAppResult(result);
  const receipt={caseId:def.id,passed:true,attempts:1,returnedBody:false};
  if(def.control) {
    requireCheck(final.status===def.appStatus&&result.code!==0&&final.error?.code===def.appError,'unexpected_control_result');
    requireCheck(observation?.appAliveBeforeAndAfter&&observation.fixturesGone&&observation.nodeGone,'cleanup_evidence_missing');
    if(def.id!=='node-self-exit')requireCheck(final.error.effect_state==='unknown','effect_state_mismatch');
    else requireCheck(final.error.effect_state===undefined||final.error.effect_state==='unknown','effect_state_mismatch');
    return {...receipt,terminal:redactedTerminalSummary(final),appErrorCode:def.appError,appErrorEffectState:final.error.effect_state??null,
      sideEffects:'unknown',cleanup:observation,
      cancellationScope:def.id==='node-cancel-frame'?'synthetic_node_stdin_frame':def.id==='node-self-exit'?'node_self_exit_connection_loss':'unawaited_native_call'};
  }
  requireCheck(final.status==='succeeded'&&result.code===0,'app_action_failed');
  const value=final.output;
  const allowed=new Set(['caseId','captured','code','stdoutUnits','stderrUnits','stdoutBytes','stderrBytes',
    'stdoutMatches','stderrMatches','cwdMatches','nodeMatches','returnedBody',
    'errorCode','effectState','methodMatches','partialOutput','messageRedacted']);
  requireCheck(value&&typeof value==='object'&&!Array.isArray(value)&&Object.keys(value).every(key=>allowed.has(key))
    &&value.caseId===def.id&&value.nodeMatches===true&&value.returnedBody===false,'author_receipt_invalid');
  if(def.error) {
    requireCheck(value.captured===false&&value.errorCode===def.error&&value.effectState==='unknown'
      &&value.methodMatches===true&&value.partialOutput===false&&value.messageRedacted===true,'capture_error_mismatch');
    return {...receipt,captured:false,errorCode:def.error,effectState:'unknown',partialOutput:false,messageRedacted:true};
  }
  requireCheck(value.captured===true&&value.code===def.code&&value.stdoutMatches===true&&value.stderrMatches===true
    &&value.cwdMatches===true,'capture_value_mismatch');
  for(const key of ['stdoutUnits','stderrUnits','stdoutBytes','stderrBytes']) {
    requireCheck(Number.isInteger(value[key])&&value[key]>=0&&value[key]<=65536,'capture_count_invalid');
  }
  requireCheck(value.stdoutBytes+value.stderrBytes<=65536,'capture_limit_mismatch');
  if(def.expectedOut!==undefined)requireCheck(value.stdoutUnits===def.expectedOut.length&&value.stdoutBytes===Buffer.byteLength(def.expectedOut),'capture_stdout_mismatch');
  if(def.expectedErr!==undefined)requireCheck(value.stderrUnits===def.expectedErr.length&&value.stderrBytes===Buffer.byteLength(def.expectedErr),'capture_stderr_mismatch');
  return {...receipt,captured:true,code:value.code,stdoutUnits:value.stdoutUnits,stderrUnits:value.stderrUnits,
    stdoutBytes:value.stdoutBytes,stderrBytes:value.stderrBytes,cwdMatches:true};
}

function environment(node) {
  const env={...process.env,OMNITRIGGER_NODE:node};
  for(const key of ['NODE_OPTIONS','OMNITRIGGER_SCRIPT_HOST','OMNITRIGGER_BROWSER_PIPE'])delete env[key];
  return env;
}
function readBounded(path,limit) {
  const file=openSync(path,'r');
  try {
    const info=fstatSync(file);
    requireCheck(info.isFile()&&info.nlink===1&&info.size<=limit,'owned_file_invalid');
    const bytes=Buffer.alloc(limit+1);let count=0;
    while(count<bytes.length) {const next=readSync(file,bytes,count,bytes.length-count,null);if(!next)break;count+=next;}
    requireCheck(count<=limit,'owned_file_limit');
    return new TextDecoder('utf-8',{fatal:true}).decode(bytes.subarray(0,count));
  } finally {closeSync(file);}
}
function atomicNew(path,text) {
  requireCheck(!existsSync(path),'owned_file_exists');
  const temporary=path+'.writing';
  writeFileSync(temporary,text,{flag:'wx'});renameSync(temporary,path);
}
function launchApp(config,layout,owned,budget) {
  const child=spawn(config.app,['--config',layout.config,'--run-action',owned.call,'--stream-action-events'],{
    cwd:layout.root,env:environment(config.node),shell:false,windowsHide:true,stdio:['ignore','pipe','pipe'],
  });
  const chunks={stdout:[],stderr:[]};let bytes=0,closed=false,timedOut=false,outputLimited=false,spawnFailed=false;
  let finish;
  const done=new Promise(resolve=>{finish=resolve;});
  const stop=()=>{if(!closed)child.kill('SIGKILL');};
  const timer=setTimeout(()=>{timedOut=true;stop();},budget);
  const hardTimer=setTimeout(()=>{
    timedOut=true;stop();child.stdout.destroy();child.stderr.destroy();child.unref();
    finish({code:null,stdout:Buffer.alloc(0),stderr:Buffer.alloc(0),timedOut,outputLimited,spawnFailed});
  },budget+5000);
  for(const stream of ['stdout','stderr'])child[stream].on('data',chunk=>{
    bytes+=chunk.length;
    if(bytes>128*1024) {outputLimited=true;stop();}
    else chunks[stream].push(chunk);
  });
  child.on('error',()=>{spawnFailed=true;});
  child.on('close',code=>{
    closed=true;clearTimeout(timer);clearTimeout(hardTimer);
    finish({code,stdout:Buffer.concat(chunks.stdout),stderr:Buffer.concat(chunks.stderr),timedOut,outputLimited,spawnFailed});
  });
  return {get pid(){return child.pid;},get closed(){return closed||child.exitCode!==null||child.signalCode!==null;},done,stop};
}

export function realAdapter() {
  return {
    kind:'real',driverPid:process.pid,now:()=>performance.now(),wallNow:()=>Date.now(),sleep:pause,
    state:pid=>{try {process.kill(pid,0);return 'alive';}catch(error){return error.code==='ESRCH'?'gone':'unknown';}},
    validate(config) {
      requireCheck(process.platform==='win32','unsupported_platform');
      verifyOrdinaryPaths([config.app,config.node,config.fixture,win32.dirname(config.root),config.root]);
      requireCheck(!existsSync(config.root),'fresh_case_root_required');
      requireCheck(lstatSync(win32.dirname(config.root)).isDirectory(),'case_parent_missing');
      for(const path of [config.app,config.node,config.fixture]) {
        requireCheck(lstatSync(path).isFile()&&win32.relative(path,realpathSync(path))==='','ordinary_exe_required');
      }
    },
    prepare(config) {
      mkdirSync(config.root);verifyOrdinaryPaths([config.root]);
      const actions=win32.join(config.root,'actions');mkdirSync(actions);
      const packageRoot=win32.join(actions,'validation-node-capture');mkdirSync(packageRoot);
      const configPath=win32.join(config.root,'bindings.json');
      writeFileSync(configPath,'[]',{flag:'wx'});
      writeFileSync(win32.join(packageRoot,'author.mjs'),AUTHOR_MODULE,{flag:'wx'});
      writeFileSync(win32.join(packageRoot,'action.json'),JSON.stringify({
        schema_version:1,id:'validation.node-capture',name:'Node采集验收',version:'1.0.0',
        actions:[{id:'run',title:'采集验收',version:1,runtime:'node',entry:'author.mjs'}],
      }),{flag:'wx'});
      return {root:config.root,config:configPath,packageRoot};
    },
    warmDirect(config,layout) {
      verifyOrdinaryPaths([config.node,config.fixture,layout.root]);
      const run=(file,args)=>spawnSync(file,args,{cwd:layout.root,env:environment(config.node),shell:false,windowsHide:true,
        timeout:config.coldTimeoutMs,maxBuffer:4096});
      const node=run(config.node,['--version']),fixture=run(config.fixture,['warmup']);
      requireCheck(!node.error&&node.status===0&&!fixture.error&&fixture.status===0,'cold_warmup_failed');
      const version=new TextDecoder('utf-8',{fatal:true}).decode(node.stdout).trim();
      const major=/^v(\d+)\.\d+\.\d+$/.exec(version);
      requireCheck(major&&Number(major[1])>=22,'node_version_unsupported');
      requireCheck(new TextDecoder('utf-8',{fatal:true}).decode(fixture.stdout)===fixtureSignature,'fixture_signature_mismatch');
      return {nodeVersion:version,fixtureSignatureVerified:true};
    },
    createCase(config,layout,def) {
      verifyOrdinaryPaths([layout.root]);
      const root=mkdtempSync(win32.join(layout.root,'capture-case-'+def.id+'-'));
      const token=randomBytes(16).toString('hex');
      const options={...def.options};
      let expectedCwd;
      if(def.cwd==='package')expectedCwd=layout.packageRoot;
      if(def.cwd==='explicit') {expectedCwd=win32.join(root,'explicit-cwd');mkdirSync(expectedCwd);options.cwd=expectedCwd;}
      const call=win32.join(root,'call.json');
      writeFileSync(call,JSON.stringify({package_id:'validation.node-capture',action_id:'run',action_version:1,
        input:actionInputForCase(def,config,{root,token},options,expectedCwd)}),{flag:'wx'});
      verifyOrdinaryPaths([root,call,config.app,config.node,config.fixture]);
      return {root,token,call};
    },
    async launch(config,layout,owned,budget) {
      const child=launchApp(config,layout,owned,budget);
      requireCheck(pid(child.pid),'app_spawn_failed');
      atomicNew(win32.join(owned.root,'app-owner.json'),JSON.stringify({schemaVersion:1,token:owned.token,app:child.pid}));
      return child;
    },
    async controlRecords(config,owned,child) {
      const record=win32.join(owned.root,'owned-pids.json'),ready=win32.join(owned.root,'fixture-ready');
      const node=win32.join(owned.root,'node-owner.json'),release=win32.join(owned.root,'release-control');
      const until=performance.now()+8000;
      while(![record,ready,node].every(existsSync)) {
        requireCheck(!child.closed&&performance.now()<until,'control_ready_unproven');await pause(5);
      }
      // 在放行自终止前批量验证和读取；时间敏感的存活观测阶段不再查询文件属性。
      verifyOrdinaryPaths([owned.root,record,ready,node,release]);
      requireCheck(readBounded(ready,128)===owned.token,'fixture_token_mismatch');
      return {record:JSON.parse(readBounded(record,2048)),nodeOwner:JSON.parse(readBounded(node,2048))};
    },
    release(owned) {atomicNew(win32.join(owned.root,'release-control'),owned.token);},
    verifyControlMarker(owned) {
      const marker=win32.join(owned.root,'node-control-request');
      verifyOrdinaryPaths([owned.root,marker]);
      requireCheck(readBounded(marker,128)===owned.token,'node_control_request_unproven');
    },
    save(layout,receipt) {
      const path=win32.join(layout.root,'receipt.json');verifyOrdinaryPaths([layout.root,path]);
      writeFileSync(path,JSON.stringify(receipt,null,2)+'\n',{flag:'wx'});
    },
  };
}

export async function runAppNodeCapture(config,adapter=realAdapter()) {
  const selected=selectedCases(config.suite??'all');
  const receipt={schemaVersion:1,tool:'app-node-capture',execution:adapter.kind,passed:false,attemptsPerCase:1,
    unknownRetried:false,bodyRecorded:false,outerActionDeadlineVerified:false,actionServiceUserCancelVerified:false,
    cancellationEvidence:'synthetic_node_cancel_frame_and_connection_loss',suite:config.suite??'all',cases:[],plannedCases:selected.length};
  let layout,current;
  try {
    adapter.validate(config);layout=adapter.prepare(config);
    receipt.warmup=await adapter.warmDirect(config,layout);
    for(const def of selected) {
      current={caseId:def.id,passed:false,attempts:1,terminal:null};
      const owned=adapter.createCase(config,layout,def);
      const child=await adapter.launch(config,layout,owned,def.cold?config.coldTimeoutMs:config.caseTimeoutMs);
      try {
        let observation;
        if(def.control) {
          const records=await adapter.controlRecords(config,owned,child);
          const identities=validateOwnedRecords(records.record,records.nodeOwner,owned,child.pid,adapter.driverPid,adapter.wallNow());
          requireCheck([identities.app,identities.node,...identities.fixtures].every(id=>adapter.state(id)==='alive'),'owned_processes_not_alive_before_release');
          adapter.release(owned);
          observation=await observeCooperativeCleanup(identities,{
            now:adapter.now,sleep:adapter.sleep,state:adapter.state,appClosed:()=>child.closed,
          },config.observeMs);
        }
        const result=await child.done;
        current.terminal=redactedTerminalSummary(parseAppResult(result));
        if(def.control)adapter.verifyControlMarker(owned);
        receipt.cases.push(evaluateResult(def,result,observation));
        current=null;
      } catch(error) {
        // 已自行退出的App若有完整终态，保存四字段摘要；不为取得终态而停止或重启App。
        if(current&&!current.terminal&&child.closed) {
          try {current.terminal=redactedTerminalSummary(parseAppResult(await child.done));} catch {}
        }
        throw error;
      } finally {
        if(!child.closed)child.stop(); // 仅本次spawn返回的App句柄；不按记录PID终止进程。
      }
    }
    receipt.passed=true;
  } catch(error) {
    receipt.error={code:error instanceof CheckFailure?error.code:'acceptance_failed'};
    if(current)receipt.cases.push({...current,errorCode:receipt.error.code,sideEffects:'unknown'});
  }
  if(layout)adapter.save(layout,receipt);
  return receipt;
}

if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  let receipt;
  try {receipt=await runAppNodeCapture(parseArguments(process.argv.slice(2)));}
  catch(error) {receipt={schemaVersion:1,passed:false,error:{code:error instanceof CheckFailure?error.code:'acceptance_failed'},bodyRecorded:false};}
  console.log(JSON.stringify(receipt));
  if(!receipt.passed)process.exitCode=1;
}
