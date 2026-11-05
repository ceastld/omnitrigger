// 只检查显示边界，不启动任何程序或系统通知。
import assert from 'node:assert/strict';
import test from 'node:test';
import {prepare,notification,complete,sanitized} from './workflow.mjs';
import {main as scriptMain} from './main-script.mjs';
import nodeMain from './main.mjs';

const task=prepare({program:'C:\\Owned\\fixture.exe',cwd:'C:\\Owned'},true);
test('输出共享UTF8总量按字节计数，stderr不显示但参与上限',()=>{
  const value=notification({code:0,stdout:'a',stderr:'中'.repeat(21845)},task);
  assert.equal(value.params.message,'a');
  assert.equal(value.summary.stderrUtf16Units,21845);
  assert.throws(()=>notification({code:0,stdout:'aa',stderr:'中'.repeat(21845)},task),error=>error.code==='process_output_too_large');
});
test('1024 UTF16显示边界不拆代理对，也不暗中裁剪',()=>{
  assert.equal(notification({code:0,stdout:'😀'.repeat(512),stderr:''},task).params.message.length,1024);
  assert.throws(()=>notification({code:0,stdout:'😀'.repeat(513),stderr:''},task),error=>error.code==='output_not_displayable');
});
test('纯文本显示保持HTML字面值，Node解码替换字符不能被误当编码证据',()=>{
  for(const text of ['<script>literal</script>','合法替换符 �'])assert.equal(notification({code:0,stdout:text,stderr:''},task).params.message,text);
});
test('不显示的stderr可以含UTF8 NUL；显示stdout则明确拒绝',()=>{
  assert.equal(notification({code:0,stdout:'ok',stderr:'\0'},task).params.message,'ok');
  assert.throws(()=>notification({code:0,stdout:'\0',stderr:''},task),error=>error.code==='output_not_displayable');
});
test('通知后的终态与错误只包含固定元数据，不回显两路内容',()=>{
  const value=notification({code:0,stdout:'PRIVATE stdout',stderr:'PRIVATE stderr'},task);
  assert.equal(JSON.stringify(complete(value,{submitted:true})).includes('PRIVATE'),false);
  const hostile=Object.defineProperty({},'code',{get:()=>{throw Error('PRIVATE getter');}});
  const error=sanitized(hostile,'ui.notify',true);
  assert.equal(error.effectState,'unknown');assert.equal(error.message.includes('PRIVATE'),false);
});

test('capture终态错误不显示部分输出，也不调用旧run回退',()=>{
  for(const code of ['process_invalid_utf8','process_output_too_large','process_cleanup_failed','process_exit_nonzero','unsupported_platform']){
    let notified=false;let fallback=false;
    const error=Object.assign(new Error('PRIVATE partial bytes'),{code,stdout:'PRIVATE stdout',stderr:'PRIVATE stderr',effectState:code==='unsupported_platform'?'not_started':'unknown'});
    const ctx={input:{program:'C:\\Owned\\fixture.exe',cwd:'C:\\Owned'},
      process:{capture:()=>{throw error;},run:()=>{fallback=true;}},ui:{notify:()=>{notified=true;}}};
    assert.throws(()=>scriptMain(ctx),fault=>fault.code===code&&!fault.message.includes('PRIVATE')
      &&!Object.hasOwn(fault,'stdout')&&!Object.hasOwn(fault,'stderr'));
    assert.equal(notified,false);assert.equal(fallback,false);
  }
});

test('缺通知接口在程序前失败；通知服务后续不可用则保留程序效果',async()=>{
  for(const entry of [scriptMain,nodeMain]){
    let executed=false;
    const run=()=>{executed=true;return{code:0,stdout:'ok',stderr:''};};
    const input={program:'C:\\Owned\\fixture.exe',cwd:'C:\\Owned'};
    try{await entry({input,process:{capture:run,run}});assert.fail('需要通知接口');}
    catch(error){assert.equal(error.code,'ui_unavailable');assert.equal(error.effectState,'not_started');}
    assert.equal(executed,false);
    try{await entry({input,process:{capture:run,run},ui:{notify:()=>{
      throw Object.assign(new Error('PRIVATE'),{code:'ui_unavailable',effectState:'not_started'});
    }}});assert.fail('通知不可用');}
    catch(error){assert.equal(error.code,'ui_unavailable');assert.equal(error.effectState,'unknown');}
    assert.equal(executed,true);
  }
});

test('调用方后续修改对象不会改变已准备程序、参数与显示策略',()=>{
  const input={program:'C:\\Owned\\fixture.exe',cwd:'C:\\Owned',args:['original'],trimOutput:false,allowNonZero:false};
  const task=prepare(input,true);
  input.args[0]='changed';input.trimOutput=true;input.allowNonZero=true;
  assert.deepEqual(task.args,['original']);assert.equal(task.trimOutput,false);assert.equal(task.allowNonZero,false);
});
