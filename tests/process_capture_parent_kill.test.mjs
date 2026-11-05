// 显式实际验收；四个受控环境变量缺失即失败，不回退系统程序或其他宿主。
import assert from 'node:assert/strict';
import {existsSync,readFileSync,writeFileSync} from 'node:fs';
import {join} from 'node:path';
import {performance} from 'node:perf_hooks';
import test from 'node:test';
import {configuration,createCase,pause,startOwnedHost,warmup} from '../tools/script-sdk-check/process-capture-test-support.mjs';
import {verifyOrdinaryPaths} from '../tools/script-sdk-check/check-clipboard-history.mjs';

function alive(pid) {
  // 信号0仅观察记录中自有夹具的存活状态，从不根据记录PID发送终止信号。
  try {process.kill(pid,0);return true;}catch(error){if(error.code==='ESRCH')return false;throw error;}
}
test('强制结束本次宿主后，私有Job内夹具及后代在自然期限前退出',{timeout:90000},async()=>{
  const config=configuration();warmup(config);
  const own=createCase(config,'parent-kill');
  const script='ctx.process.capture(ctx.input.program,["descendant",ctx.input.root,ctx.input.token],{timeoutMs:60000});return {unexpectedCompletion:true};';
  const host=startOwnedHost(config,own.root,script,{program:config.fixture,root:own.root,token:own.token});
  let ids=[];
  try {
    const ready=join(own.root,'fixture-ready');
    const deadline=performance.now()+8000;
    let prepared=false;
    while(performance.now()<deadline) {
      assert(!host.spawnError&&!host.closed,'受控宿主在夹具就绪前退出');
      if(existsSync(ready)) {
        verifyOrdinaryPaths([ready]);
        if(readFileSync(ready,'utf8')===own.token){prepared=true;break;}
      }
      await pause(20);
    }
    assert(prepared,'未取得本次夹具的就绪标记');
    const recordPath=join(own.root,'owned-pids.json');verifyOrdinaryPaths([recordPath]);
    const record=JSON.parse(readFileSync(recordPath,'utf8'));
    assert.equal(record.schemaVersion,1);assert.equal(record.token,own.token);
    ids=[record.fixture,record.descendant];
    assert(ids.every(pid=>Number.isSafeInteger(pid)&&pid>0&&pid<=0xffffffff));
    assert.notEqual(ids[0],ids[1]);
    assert(ids.every(pid=>pid!==process.pid&&pid!==host.child.pid));
    assert(Number.isSafeInteger(record.startedAtUnixMs));
    const age=Date.now()-record.startedAtUnixMs;
    assert(age>=0&&age<5000,'必须在15秒自然退出前留足独立验证窗口');
    assert(ids.every(alive),'两个自有夹具须在终止宿主前确实存活');
    const killedAt=performance.now();await host.stop();
    while(ids.some(alive)&&performance.now()-killedAt<2000)await pause(20);
    const exitLatencyMs=Math.round(performance.now()-killedAt);
    assert(ids.every(pid=>!alive(pid)),'直接结束宿主不能替代Job内后代已退出的证据');
    assert(exitLatencyMs<2000,'须早于夹具自然退出确认Job效果');
    writeFileSync(join(own.root,'parent-kill-result.json'),JSON.stringify({passed:true,engine:config.engine,ownedProcessCount:2,exitLatencyMs}),{flag:'wx'});
  } finally {
    await host.stop();
    // 失败时只等有界夹具自然退出，绝不改为根据PID杀任意进程。
    const deadline=performance.now()+16000;
    while(ids.length&&ids.some(alive)&&performance.now()<deadline)await pause(50);
  }
});
