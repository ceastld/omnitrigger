// 合成作者入口；工具只复制生成的源码，导入本文件不会运行任何动作。
export function createCaptureAuthor(platform) {
  const proc=platform.proc;
  const normalize=path=>{
    const slash=String.fromCharCode(92);
    let value=String(path).split('/').join(slash);
    if(value.startsWith(slash+slash+'?'+slash))value=value.slice(4);
    while(value.endsWith(slash))value=value.slice(0,-1);
    return value.toLowerCase();
  };
  const fail=code=>{throw Object.assign(Error('合成采集验收入口未完成。'),{code});};
  const wait=async predicate=>{
    const deadline=platform.now()+8000;
    while(!predicate()) {
      if(platform.now()>=deadline)fail('author_ready_timeout');
      await platform.sleep(2);
    }
  };
  return async ctx=>{
    const input=ctx.input;
    if(typeof ctx.process.capture!=='function')fail('capture_unavailable');
    if(normalize(proc.execPath)!==normalize(input.expectedNode))fail('unexpected_node');
    const control=['node-self-exit','node-cancel-frame','unawaited'].includes(input.caseId);
    if(control) {
      await wait(()=>platform.exists(platform.join(input.root,'app-owner.json')));
      const owner=JSON.parse(platform.read(platform.join(input.root,'app-owner.json')));
      if(owner.token!==input.token||owner.app!==proc.ppid)fail('author_owner_mismatch');
      const pending=ctx.process.capture(input.program,['descendant',input.root,input.token],{timeoutMs:10000});
      // 已有显式拒绝仍保持 rejected；不让准备阶段出现未处理拒绝。
      pending.catch(()=>{});
      await wait(()=>platform.exists(platform.join(input.root,'fixture-ready')));
      if(platform.read(platform.join(input.root,'fixture-ready'))!==input.token)fail('author_fixture_mismatch');
      platform.writeNew(platform.join(input.root,'node-owner.json'),JSON.stringify({
        schemaVersion:1,token:input.token,node:proc.pid,app:proc.ppid,
      }));
      await wait(()=>platform.exists(platform.join(input.root,'release-control')));
      if(platform.read(platform.join(input.root,'release-control'))!==input.token)fail('author_release_mismatch');
      platform.writeNew(platform.join(input.root,'node-control-request'),input.token);
      if(input.caseId==='node-self-exit') {
        // 只允许终止作者自己的当前Node进程；不使用任何记录中的PID。
        proc.kill(proc.pid,'SIGKILL');
        fail('self_exit_returned');
      }
      if(input.caseId==='node-cancel-frame') {
        // 合成Node宿主既有stdin取消帧；不冒称用户界面/ActionService取消。
        platform.cancelHost();
        await pending;
        fail('cancel_frame_not_effective');
      }
      return {caseId:input.caseId,returnedWithoutAwait:true};
    }
    try {
      const value=await ctx.process.capture(input.program,input.args,input.options);
      const expectedOut=input.expectedOut??(input.expectedOutChar===undefined?undefined:input.expectedOutChar.repeat(input.expectedOutUnits));
      const expectedErr=input.expectedErr??(input.expectedErrChar===undefined?undefined:input.expectedErrChar.repeat(input.expectedErrUnits));
      return {caseId:input.caseId,captured:true,code:value.code,
        stdoutUnits:value.stdout.length,stderrUnits:value.stderr.length,
        stdoutBytes:platform.byteLength(value.stdout),stderrBytes:platform.byteLength(value.stderr),
        stdoutMatches:expectedOut===undefined||value.stdout===expectedOut,
        stderrMatches:expectedErr===undefined||value.stderr===expectedErr,
        cwdMatches:input.expectedCwd===undefined||normalize(value.stdout)===normalize(input.expectedCwd),
        nodeMatches:true,returnedBody:false};
    } catch(error) {
      const known=['process_output_too_large','process_invalid_utf8','process_exit_nonzero',
        'process_timeout','cancelled','timeout','process_cleanup_failed','native_bridge_unavailable',
        'unsupported_platform','invalid_input','invalid_arguments'];
      return {caseId:input.caseId,captured:false,
        errorCode:known.includes(error?.code)?error.code:'unrecognized_capture_error',
        effectState:['not_started','unknown'].includes(error?.effectState)?error.effectState:null,
        methodMatches:error?.method==='process.capture',
        partialOutput:error?.stdout!==undefined||error?.stderr!==undefined,
        messageRedacted:!String(error?.message).includes('controlled output')&&!String(error?.message).includes(input.token),
        nodeMatches:true,returnedBody:false};
    }
  };
}

export const AUTHOR_MODULE=[
  'import {existsSync,readFileSync,writeFileSync} from "node:fs";',
  'import {join} from "node:path";',
  'const createCaptureAuthor='+createCaptureAuthor.toString()+';',
  'export default createCaptureAuthor({proc:process,join,now:()=>performance.now(),',
  'sleep:ms=>new Promise(done=>setTimeout(done,ms)),exists:existsSync,',
  'read:path=>readFileSync(path,"utf8"),writeNew:(path,text)=>writeFileSync(path,text,{flag:"wx"}),',
  'byteLength:text=>Buffer.byteLength(text),cancelHost:()=>process.stdin.emit("data",Buffer.from("cancel\\n"))});',
].join('\n');
