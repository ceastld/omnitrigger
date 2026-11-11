import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const source=readFileSync(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const sdk=readFileSync(new URL('../crates/omnitrigger-actions/runtime/actions-api.js',import.meta.url),'utf8')+'\n'+readFileSync(new URL('../crates/omnitrigger-actions/runtime/state-api.js',import.meta.url),'utf8')+'\n'+readFileSync(new URL('../tools/js-engine-probe/src/sdk.js',import.meta.url),'utf8');
const options={expectedProcessId:20,expectedProcessCreatedFileTime:'18446744073709550000',validForMs:500};
const original={window_handle:'101',process_id:10,thread_id:11,process_created_file_time:'134000000000000001',expires_at_unix_ms:1000};
const target={capture_id:1,context:{window_handle:'9007199254740993',process_id:20,thread_id:21,
  process_created_file_time:options.expectedProcessCreatedFileTime,expires_at_unix_ms:1000}};
const plain=value=>JSON.parse(JSON.stringify(value));
function api(invoke){const context=vm.createContext({invoke});return vm.runInContext(source+'\ncreateDesktopApi(invoke).desktop',context);}
function fault(run,code,method,effect='not_started'){
  assert.throws(run,error=>error.code===code&&error.method===method&&error.effectState===effect);
}

for(const async of [false,true])test(`${async?'异步':'同步'}捕获与发送保留精度、原目标及单一桥接`,async()=>{
  const calls=[];
  const invoke=(method,params)=>{calls.push({method,params:plain(params)});
    const value=method==='target.context'?original:method==='target.capture_foreground'?target:{submitted:true};
    return async?Promise.resolve(value):value;
  };
  const desktop=api(invoke);
  const before=await desktop.targetContext();
  const lease=await desktop.captureForegroundTarget(options);
  assert(Object.isFrozen(lease)&&Object.isFrozen(lease.context));
  await desktop.sendShortcutTo(lease,[163,76],{keyDownDelayMs:7});
  await desktop.sendTextTo(lease,'合成文本');
  assert.equal((await desktop.targetContext()).window_handle,before.window_handle);
  assert.deepEqual(calls.map(c=>c.method),['target.context','target.capture_foreground','target.keyboard.shortcut','target.keyboard.text','target.context']);
  assert.deepEqual(calls[1].params,{expected_process_id:20,expected_process_created_file_time:options.expectedProcessCreatedFileTime,valid_for_ms:500});
  assert.deepEqual(calls[2].params,{target,input:{keys:[163,76],key_down_delay_ms:7}});
  assert.deepEqual(calls[3].params,{target,input:{text:'合成文本'}});
});

test('无效捕获参数在桥接前拒绝',()=>{
  let calls=0;const desktop=api(()=>{calls++;throw Error('禁止派发');});
  for(const value of [null,{}, {...options,expectedProcessId:0},{...options,expectedProcessId:4294967296},
    {...options,expectedProcessCreatedFileTime:123},{...options,expectedProcessCreatedFileTime:'01'},
    {...options,expectedProcessCreatedFileTime:'18446744073709551616'}, {...options,validForMs:0},
    {...options,validForMs:5001},{...options,extra:true}]){
    fault(()=>desktop.captureForegroundTarget(value),'invalid_input','target.capture_foreground');
  }
  assert.equal(calls,0);
});

test('捕获回执须完整且与预期进程身份一致',()=>{
  for(const value of [{}, {...target,extra:true},{...target,capture_id:0},{...target,capture_id:129},
    {...target,context:{...target.context,process_id:99}},
    {...target,context:{...target.context,process_created_file_time:'2'}},
    {...target,context:{...target.context,window_handle:9007199254740992}},
    {...target,context:{...target.context,expires_at_unix_ms:0}}]){
    fault(()=>api(()=>value).captureForegroundTarget(options),'invalid_response','target.capture_foreground');
  }
});

test('旧上下文和修改后的无效引用不能发送',()=>{
  let calls=0;const desktop=api(()=>{calls++;return{submitted:true};});
  for(const value of [original,{...target,capture_id:0},{...target,context:{...target.context,window_handle:'0'}}]){
    fault(()=>desktop.sendTextTo(value,'x'),'invalid_input','target.keyboard.text');
  }
  assert.equal(calls,0);
});

test('按键、文本与时序继续使用原输入校验',()=>{
  let calls=0;const desktop=api(()=>{calls++;return{submitted:true};});
  for(const run of [()=>desktop.sendShortcutTo(target,[]),()=>desktop.sendShortcutTo(target,[0]),
    ()=>desktop.sendShortcutTo(target,[163,76],{keyDownDelayMs:1001}),
    ()=>desktop.sendShortcutTo(target,[163,76],{timing:{}})]){
    fault(run,'invalid_input','target.keyboard.shortcut');
  }
  for(const text of ['\0','\ud800'])fault(()=>desktop.sendTextTo(target,text),'invalid_input','target.keyboard.text');
  assert.equal(calls,0);
});

for(const method of ['shortcut','text'])test(`${method}未确认提交不能报告成功`,()=>{
  for(const reply of [null,{}, {submitted:false},{submitted:true,extra:true}]){
    const desktop=api(()=>reply);
    fault(()=>method==='text'?desktop.sendTextTo(target,'x'):desktop.sendShortcutTo(target,[163,76]),
      'invalid_response',`target.keyboard.${method}`,'unknown');
  }
});

for(const async of [false,true])test(`${async?'异步':'同步'}桥接错误保持方法及提交范围`,async()=>{
  for(const effect of ['not_started','unknown']){
    const desktop=api(()=>{const error=Object.assign(new Error('不应转发的底层消息'),{code:'target_changed',effectState:effect});
      if(async)return Promise.reject(error);throw error;
    });
    if(async)await assert.rejects(desktop.sendTextTo(target,'x'),error=>error.code==='target_changed'&&error.method==='target.keyboard.text'&&error.effectState===effect&&!error.message.includes('底层消息'));
    else fault(()=>desktop.sendTextTo(target,'x'),'target_changed','target.keyboard.text',effect);
  }
});

test('新旧方法和delay共享既有轻量宿主总预算',()=>{
  let calls=0;
  const context=vm.createContext({ctx:{input:{},invocation:{},__nativeContext:{connection:{}}},
    createBrowserApi:()=>({}),createClipboardHistoryApi:()=>({}),createProcessCaptureApi:()=>({}),
    __hostCall:serialized=>{calls++;const request=JSON.parse(serialized);
      return JSON.stringify({ok:true,value:request.kind==='native'?(request.method==='target.capture_foreground'?target:original):null});
    }});
  vm.runInContext(source+'\n'+sdk+'\nglobalThis.testContext=ctx;',context);
  const ctx=context.testContext;ctx.delay(0);
  for(let i=0;i<30;i++)ctx.desktop.targetContext();
  const lease=ctx.desktop.captureForegroundTarget(options);assert.equal(calls,32);
  assert.throws(()=>ctx.desktop.sendTextTo(lease,'x'));
  assert.equal(calls,32);
});
