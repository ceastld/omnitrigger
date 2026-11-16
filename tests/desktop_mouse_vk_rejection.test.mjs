import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const source=readFileSync(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const mouseVks=[1,2,4,5,6];
const original={window_handle:'101',process_id:10,thread_id:11,process_created_file_time:'134000000000000001',expires_at_unix_ms:1000};
const options={expectedProcessId:20,expectedProcessCreatedFileTime:'18446744073709550000',validForMs:500};
const captured={capture_id:1,context:{window_handle:'9007199254740993',process_id:20,thread_id:21,
  process_created_file_time:options.expectedProcessCreatedFileTime,expires_at_unix_ms:1000}};
const plain=value=>JSON.parse(JSON.stringify(value));
function fixture(asynchronous) {
  const calls=[];
  const invoke=(method,params)=>{
    calls.push({method,params:plain(params)});
    const value=method==='target.capture_foreground'?captured:method==='target.context'?original:{submitted:true};
    return asynchronous?Promise.resolve(value):value;
  };
  const context=vm.createContext({invoke});
  return {calls,desktop:vm.runInContext(source+'\ncreateDesktopApi(invoke).desktop',context)};
}
function invalid(run,method) {
  assert.throws(run,error=>error.code==='invalid_input'&&error.effectState==='not_started'&&error.method===method);
}
for(const asynchronous of [false,true]) {
  test((asynchronous?'Promise bridge':'sync bridge')+' rejects mouse VK before every keyboard route',async()=>{
    const {calls,desktop}=fixture(asynchronous);
    const target=await desktop.captureForegroundTarget(options);
    calls.length=0;
    for(const key of mouseVks) {
      invalid(()=>desktop.sendShortcut([key]),'keyboard.shortcut');
      invalid(()=>desktop.sendShortcut([162,key],{timing:{key_down_delay_ms:0}}),'keyboard.shortcut');
      invalid(()=>desktop.sendKeySequence([],[key]),'keyboard.sequence');
      invalid(()=>desktop.sendKeySequence([key],[65]),'keyboard.sequence');
      invalid(()=>desktop.sendShortcutTo(target,[key]),'target.keyboard.shortcut');
    }
    assert.equal(calls.length,0,'invalid mouse codes must not reach Native');
  });
  test((asynchronous?'Promise bridge':'sync bridge')+' preserves keyboard order and captured target identity',async()=>{
    const {calls,desktop}=fixture(asynchronous);
    const target=await desktop.captureForegroundTarget(options);
    const before=plain(await desktop.targetContext());
    calls.length=0;
    await desktop.sendShortcut([162,65]);
    await desktop.sendKeySequence([162],[65,66]);
    await desktop.sendShortcutTo(target,[163,76],{keyDownDelayMs:7});
    const after=plain(await desktop.targetContext());
    assert.deepEqual(after,before);
    assert.deepEqual(calls,[
      {method:'keyboard.shortcut',params:{keys:[162,65]}},
      {method:'keyboard.sequence',params:{modifiers:[162],keys:[65,66],key_down_delay_ms:0}},
      {method:'target.keyboard.shortcut',params:{target:captured,input:{keys:[163,76],key_down_delay_ms:7}}},
      {method:'target.context',params:{}},
    ]);
  });
}
