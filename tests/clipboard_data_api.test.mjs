import fs from 'node:fs';
import vm from 'node:vm';
import {EventEmitter} from 'node:events';
// 仅调用共享公开 factory 与内存 invoke；不启动脚本宿主、不读取系统剪贴板、不注入输入。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';

const source = readFileSync(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js', import.meta.url), 'utf8');
const createApi = runInNewContext(source + '\ncreateDesktopApi');
const plain = value => JSON.parse(JSON.stringify(value));
const privateText = 'synthetic_clipboard_payload_只限内存_😀';
const sequence = 0xffffffff;
const defaults = {timeout_ms:600, poll_interval_ms:15, settle_ms:30, allow_unchanged:false};
const copyMethod = 'selection.copy_data';
const readMethod = 'clipboard.read_data';
const snapshot = (kind, copy = false) => kind === 'text'
  ? {kind, text:privateText, sequenceNumber:sequence, ...(copy ? {changed:true} : {})}
  : {kind, paths:['X:\\SyntheticOnly\\one.txt', '\\\\synthetic.invalid\\share\\中文😀.txt'], sequenceNumber:0, ...(copy ? {changed:false} : {})};

function api(reply, asynchronous = false) {
  const calls = [];
  const result = createApi((method, params) => {
    calls.push([method, plain(params)]);
    assert([readMethod, copyMethod].includes(method), '只能调用新格式API，不能降级到旧文本/磁盘/输入API');
    if (reply instanceof Error) {
      if (asynchronous) return Promise.reject(reply);
      throw reply;
    }
    return asynchronous ? Promise.resolve(reply) : reply;
  });
  return {result, calls};
}
function structured(method, effect, code) {
  return error => typeof error?.code === 'string' && /^[a-z][a-z0-9_]*$/.test(error.code)
    && (code === undefined || error.code === code) && error.method === method && error.effectState === effect
    && !String(error.message).includes(privateText) && !String(error.message).includes('SyntheticOnly')
    && !String(error.message).includes('synthetic.invalid');
}
const methodCall = (result, copy, options) => copy ? result.desktop.copySelectionData(options) : result.clipboard.readData();

test('公开 factory 提供两个新API，并保留已有文本API', () => {
  const {result} = api(snapshot('text'));
  assert.equal(typeof result.clipboard.readData, 'function');
  assert.equal(typeof result.desktop.copySelectionData, 'function');
  assert.equal(typeof result.clipboard.readText, 'function');
  assert.equal(typeof result.clipboard.writeText, 'function');
  assert.equal(typeof result.desktop.copySelectionText, 'function');
});

test('同步与Promise适配都保持文本/文件联合、空文本/空列表、u32边界和原始字符', async () => {
  for (const asynchronous of [false, true]) for (const copy of [false, true]) {
    for (const value of [snapshot('text', copy), snapshot('files', copy),
      {kind:'text',text:'',sequenceNumber:0,...(copy ? {changed:false} : {})},
      {kind:'text',text:' \r\n合成😀 ',sequenceNumber:1,...(copy ? {changed:true} : {})},
      {kind:'files',paths:[],sequenceNumber:sequence,...(copy ? {changed:false} : {})}]) {
      const {result,calls} = api(value, asynchronous);
      const returned = methodCall(result, copy, copy ? {allowUnchanged:true} : undefined);
      assert.equal(typeof returned?.then === 'function', asynchronous);
      assert.deepEqual(plain(await returned), value);
      assert.equal(calls.length, 1);
      assert.equal(calls[0][0], copy ? copyMethod : readMethod);
      if (!copy) assert.deepEqual(calls[0][1], {});
    }
  }
});

test('文件路径仅作为数据保留，不执行本地路径校验、stat、枚举或打开', async () => {
  const value = {kind:'files',paths:['relative synthetic.txt','C:\\OneDrive-synthetic\\file.txt',
    '\\\\synthetic.invalid\\share\\file.txt','X:\\a & literal.txt','  literal whitespace  ',' '],sequenceNumber:27};
  for (const asynchronous of [false,true]) {
    const {result,calls}=api(value,asynchronous);
    assert.deepEqual(plain(await result.clipboard.readData()),value);
    assert.deepEqual(calls,[[readMethod,{}]]);
  }
});

test('copy选项仅camelCase输入，明确映射snake_case；默认不夹紧或改写预算', () => {
  for (const options of [undefined, {}]) {
    const {result,calls}=api(snapshot('text',true));
    result.desktop.copySelectionData(options);
    assert.equal(calls[0][0],copyMethod);
    const params=calls[0][1];
    assert(Object.keys(params).every(key => Object.hasOwn(defaults,key)));
    assert.deepEqual({...defaults,...params},defaults);
  }
  for (const [options,expected] of [
    [{timeoutMs:400,pollIntervalMs:10,settleMs:0,allowUnchanged:true},{timeout_ms:400,poll_interval_ms:10,settle_ms:0,allow_unchanged:true}],
    [{timeoutMs:1,pollIntervalMs:1,settleMs:1000,allowUnchanged:false},{timeout_ms:1,poll_interval_ms:1,settle_ms:1000,allow_unchanged:false}],
    [{timeoutMs:5000,pollIntervalMs:1000,settleMs:1000,allowUnchanged:true},{timeout_ms:5000,poll_interval_ms:1000,settle_ms:1000,allow_unchanged:true}],
  ]) {
    const {result,calls}=api(snapshot('text',true));
    result.desktop.copySelectionData(options);
    assert.deepEqual(calls,[[copyMethod,expected]]);
  }
});

test('非法选项在任何invoke前拒绝，错误不回显正文', () => {
  const values=[null,true,1,'options',[],new String('options'),
    {extra:privateText},{timeout_ms:400},{poll_interval_ms:10},{settle_ms:0},{allow_unchanged:true},
    {timeoutMs:1}, {timeoutMs:10,pollIntervalMs:11}, {[Symbol('unknown')]:privateText}];
  for (const [name,bad] of [
    ['timeoutMs',[null,false,'400',0,-1,5001,0.5,NaN,Infinity]],
    ['pollIntervalMs',[null,false,'10',0,-1,1001,1.5,NaN,Infinity]],
    ['settleMs',[null,false,'0',-1,1001,1.5,NaN,Infinity]],
    ['allowUnchanged',[null,0,1,'true',{},[]]],
  ]) for (const value of bad) values.push({[name]:value});
  for (const options of values) {
    const {result,calls}=api(snapshot('text',true));
    assert.throws(()=>result.desktop.copySelectionData(options),structured(copyMethod,'not_started','invalid_input'));
    assert.deepEqual(calls,[]);
  }
  for (const args of [[{}],[undefined],[null],[privateText]]) {
    const {result,calls}=api(snapshot('text'));
    assert.throws(()=>result.clipboard.readData(...args),structured(readMethod,'not_started','invalid_input'));
    assert.deepEqual(calls,[]);
  }
  const {result,calls}=api(snapshot('text',true));
  assert.throws(()=>result.desktop.copySelectionData({},{}),structured(copyMethod,'not_started','invalid_input'));
  assert.deepEqual(calls,[]);
});

test('结果UTF16上限精确计算：65536单元、64项及总路径长度边界', async () => {
  for (const copy of [false,true]) for (const asynchronous of [false,true]) {
    for (const value of [
      {kind:'text',text:'x'.repeat(65536),sequenceNumber:1},
      {kind:'text',text:'😀'.repeat(32768),sequenceNumber:2},
      {kind:'files',paths:Array(64).fill('x'.repeat(1024)),sequenceNumber:3},
      {kind:'files',paths:['😀'.repeat(32768)],sequenceNumber:4},
    ]) {
      if (copy) value.changed=true;
      const {result}=api(value,asynchronous);
      assert.deepEqual(plain(await methodCall(result,copy)),value);
    }
  }
});

function invalidResults(copy) {
  const text={kind:'text',text:privateText,sequenceNumber:1,...(copy ? {changed:true} : {})};
  const files={kind:'files',paths:['X:\\SyntheticOnly\\file.txt'],sequenceNumber:1,...(copy ? {changed:true} : {})};
  const values=[undefined,null,true,[],{},privateText,
    {...text,kind:'image'}, {...text,extra:privateText}, {...text,paths:[]}, {...files,text:privateText},
    {...text,text:1}, {...text,text:null}, {...text,text:'\ud800'}, {...text,text:'x\udc00'}, {...text,text:'x\0y'},
    {...text,text:'x'.repeat(65537)}, {...text,text:'😀'.repeat(32769)},
    {...files,paths:null}, {...files,paths:privateText}, {...files,paths:[1]}, {...files,paths:[null]},
    {...files,paths:['']}, {...files,paths:['x\0y']}, {...files,paths:['\ud800']}, {...files,paths:Array(65).fill('x')},
    {...files,paths:Array(64).fill('x'.repeat(1024)).concat('x')},
    {...files,paths:['x'.repeat(32768),'x'.repeat(32769)]},
    {...files,paths:['😀'.repeat(32769)]}, {...text,[Symbol('extra')]:privateText}];
  for (const sequenceNumber of [-1,4294967296,0.5,'1',null,true,NaN,Infinity]) values.push({...text,sequenceNumber});
  const missingSequence={...text}; delete missingSequence.sequenceNumber; values.push(missingSequence);
  if (copy) {
    const missing={...text};delete missing.changed;values.push(missing);
    for (const changed of [null,0,1,'true',{},[]]) values.push({...text,changed});
  } else values.push({...text,changed:true});
  return values;
}

test('畸形联合、额外字段、坏Unicode、数量和序号不转成空文本或文件列表', async () => {
  for (const copy of [false,true]) for (const asynchronous of [false,true]) for (const value of invalidResults(copy)) {
    const {result,calls}=api(value,asynchronous);
    await assert.rejects(async()=>await methodCall(result,copy),structured(copy ? copyMethod : readMethod,copy ? 'unknown' : 'not_started','invalid_response'));
    assert.equal(calls.length,1,'坏回执不得重试、降级或重新读取');
  }
});

test('结构化平台错误保留方法/阶段，去除payload；不降级读取旧文本', async () => {
  for (const copy of [false,true]) for (const asynchronous of [false,true]) {
    for (const [code,effect] of [['native_bridge_unavailable','not_started'],['unsupported_clipboard_format',copy?'unknown':'not_started'],
      ['cancelled',copy?'unknown':'not_started'],['timeout',copy?'unknown':'not_started'],
      ...(copy ? [['selection_unchanged','unknown'],['target_changed','not_started']] : [])]) {
      const error=Object.assign(new Error(privateText+' X:\\SyntheticOnly\\file.txt'),{code,method:'wrong.method',effectState:effect});
      const {result,calls}=api(error,asynchronous);
      await assert.rejects(async()=>await methodCall(result,copy),structured(copy?copyMethod:readMethod,effect,code));
      assert.equal(calls.length,1);
    }
  }
});

test('新的轻量示例显式单实例、read/copy调用与默认摘要契约', () => {
  const root=new URL('../examples/actions/selection-data/',import.meta.url);
  const manifest=JSON.parse(readFileSync(new URL('action.json',root),'utf8'));
  assert.equal(manifest.id,'examples.selection-data');
  assert.deepEqual(manifest.actions,[{id:'summarize',title:'查看选区或剪贴板数据摘要',version:1,runtime:'script',entry:'main.js',single_instance:true}]);
  for (const [file,mode] of [['call.json','read'],['copy-call.json','copy']]) {
    const call=JSON.parse(readFileSync(new URL(file,root),'utf8'));
    assert.equal(call.package_id,manifest.id);assert.equal(call.action_id,'summarize');assert.equal(call.action_version,1);
    assert.equal(call.input.mode,mode);
  }
  const entry=readFileSync(new URL('main.js',root),'utf8');
  assert(entry.includes('ctx.clipboard.readData()') && entry.includes('ctx.desktop.copySelectionData('));
  assert(!entry.includes('console.') && !entry.includes('ctx.open(') && !entry.includes('ctx.fs.'));
});

test('复制格式：同步和Promise显式选择格式，auto保留默认wire', async () => {
  for (const asynchronous of [false,true]) {
    for (const format of ['auto','text','files']) {
      const value=format==='files'?{...snapshot('files',true),changed:true}:{kind:'text',text:'',sequenceNumber:0,changed:true};
      const {result,calls}=api(value,asynchronous);
      assert.deepEqual(plain(await result.desktop.copySelectionData({format})),value);
      assert.deepEqual(calls,[[copyMethod,{...defaults,...(format==='auto'?{}:{format})}]]);
    }
    const {result,calls}=api(snapshot('files',true),asynchronous);
    await result.desktop.copySelectionData({format:'files',timeoutMs:400,pollIntervalMs:10,settleMs:0,allowUnchanged:true});
    assert.deepEqual(calls,[[copyMethod,{format:'files',timeout_ms:400,poll_interval_ms:10,settle_ms:0,allow_unchanged:true}]]);
  }
});

test('复制格式：坏参数和readData格式参数在invoke前拒绝', async () => {
  for (const asynchronous of [false,true]) {
    const trap=Object.defineProperty({},'format',{enumerable:true,get(){throw Error(privateText);}});
    for (const options of [{format:null},{format:1},{format:''},{format:'AUTO'},{format:{}},trap]) {
      const {result,calls}=api(snapshot('text',true),asynchronous);
      await assert.rejects(async()=>result.desktop.copySelectionData(options),structured(copyMethod,'not_started','invalid_input'));
      assert.deepEqual(calls,[]);
    }
    const {result,calls}=api(snapshot('files'),asynchronous);
    await assert.rejects(async()=>result.clipboard.readData({format:'files'}),structured(readMethod,'not_started','invalid_input'));
    assert.deepEqual(calls,[]);
  }
});

test('复制格式：回执种类不符明确失败且不重试或降级', async () => {
  for (const asynchronous of [false,true]) {
    for (const [format,kind] of [['files','text'],['text','files']]) {
      const {result,calls}=api(snapshot(kind,true),asynchronous);
      await assert.rejects(async()=>result.desktop.copySelectionData({format}),structured(copyMethod,'unknown','invalid_response'));
      assert.equal(calls.length,1);
      assert.equal(calls[0][1].format,format);
    }
    const {result,calls}=api(Object.assign(Error(privateText),{code:'unsupported_clipboard_format',effectState:'unknown'}),asynchronous);
    await assert.rejects(async()=>result.desktop.copySelectionData({format:'files'}),structured(copyMethod,'unknown','unsupported_clipboard_format'));
    assert.equal(calls.length,1);
  }
});

// writeFiles 的既有八组端口合同；只使用真实源码与内存替身。
test('文件列表写入 SDK 端口合同', async t => {
const here=new URL('../',import.meta.url);
const source=path=>fs.readFileSync(new URL(path,here),'utf8');
const desktop=source('crates/omnitrigger-actions/runtime/desktop-api.js'),host=source('crates/omnitrigger-actions/runtime/host.mjs'),sdk=source('crates/omnitrigger-actions/runtime/actions-api.js')+'\n'+source('crates/omnitrigger-actions/runtime/state-api.js')+'\n'+source('tools/js-engine-probe/src/sdk.js');
const asyncLine=host.split('\n').find(line=>line.startsWith('const asyncApi ='));
const ok={written:true,sequenceNumber:0xffffffff},paths=['C:\\合成😀.txt','\\\\server\\share\\a.txt','C:\\合成😀.txt'];
const plain=value=>JSON.parse(JSON.stringify(value));let passed=0;
async function test(name,fn){await t.test(name,fn);passed++;}
const predicate=(code,effect)=>error=>{assert.equal(error.code,code);assert.equal(error.effectState,effect);assert.equal(error.method,'clipboard.write_files');assert(!error.message.includes('private'));assert.equal(error.requiredPermissions,undefined);return true;};
function shared(invoke,async=false){const context=vm.createContext({invoke});vm.runInContext(desktop+'\n'+asyncLine+'\nglobalThis.api='+(async?'asyncApi(createDesktopApi(invoke).clipboard)':'createDesktopApi(invoke).clipboard')+';',context);return context.api;}
function sync({bridge=true,reply=()=>ok}={}){
 const calls=[];const context=vm.createContext({ctx:bridge?{__nativeContext:{connection:{port:9,token:'a'.repeat(64)}}}:{},__hostCall:serialized=>{const request=JSON.parse(serialized);calls.push(request);return JSON.stringify({ok:true,value:reply(request)});},createProcessCaptureApi:()=>({capture(){}}),createBrowserApi:()=>({}),createClipboardHistoryApi:()=>({})});
 vm.runInContext(desktop+'\n'+sdk,context);return{api:context.ctx.clipboard,calls};
}
function node({bridge=true,mode='ok',value=ok,error}={}){
 const calls=[],sockets=[],timers=new Map();
 class Socket extends EventEmitter{setEncoding(){}setTimeout(_ms,fn){this.timeout=fn;}destroy(){}write(frame){const request=JSON.parse(frame);calls.push(request);if(mode==='ok')queueMicrotask(()=>this.emit('data',JSON.stringify(error?{id:request.id,ok:false,error}:{id:request.id,ok:true,value})+'\n'));}}
 const controller=new AbortController();const context=vm.createContext({request:bridge?{native:{port:9,token:'a'.repeat(64)}}:{},controller,Buffer,fault:(code,message)=>Object.assign(new Error(message??code),{code}),checkCancelled:()=>{if(controller.signal.aborted)throw controller.signal.reason;},createConnection:()=>{const socket=new Socket();sockets.push(socket);queueMicrotask(()=>mode==='pre_error'?socket.emit('error',new Error('private')):socket.emit('connect'));return socket;},setTimeout:callback=>{const id={};timers.set(id,callback);return id;},clearTimeout:id=>timers.delete(id)});
 const m=host.indexOf('const nativeMethods ='),methods=host.slice(m,host.indexOf(']);',m)+3);const f=host.indexOf('function nativeCall('),native=host.slice(f,host.indexOf('\n}',f)+2);
 vm.runInContext('let nativeId=0,nativePending,nativePendingMethod;const nativeSockets=new Set();\n'+methods+'\n'+native+'\n'+desktop+'\n'+asyncLine+'\nglobalThis.api=asyncApi(createDesktopApi(nativeCall).clipboard);',context);
 return{api:context.api,calls,sockets,timers,controller};
}
await test('共享SDK同步与异步保持Unicode路径顺序重复项和窄回执',async()=>{
 for(const asynchronous of [false,true]){const calls=[];const api=shared((method,params)=>{calls.push({method,params});return asynchronous?Promise.resolve(ok):ok;},asynchronous);const result=await api.writeFiles(Object.freeze([...paths]));assert.deepEqual(plain(result),ok);assert(Object.isFrozen(result));assert.deepEqual(plain(calls),[{method:'clipboard.write_files',params:{paths}}]);}
});
await test('全部坏参数在零桥接时拒绝且不回显路径',async()=>{
 let calls=0;const api=shared(()=>{calls++;return ok;},true);const sparse=Array(1);const extra=['C:\\a'];extra.extra=1;const getter=Object.defineProperty(['C:\\a'],'0',{get(){throw Error('private');}});
 for(const args of [[],[[]],[['relative']],[[null]],[['C:relative']],[['\\\\?\\C:\\a']],[['C:\\..\\a']],[['C:\\a:stream']],[['C:\\bad\0a']],[['C:\\bad\ud800']],[sparse],[extra],[getter],[['C:\\a'],{}],[Array(65).fill('C:\\a')],[['C:\\'+'😀'.repeat(16383)]],[Array(4).fill('C:\\'+'x'.repeat(16382))]])await assert.rejects(api.writeFiles(...args),predicate('invalid_input','not_started'));
 assert.equal(calls,0);
});
await test('请求发送使用独立完整快照而非调用方后续变更',async()=>{
 let params;const api=shared((_method,p)=>{params=p;return Promise.resolve(ok);},true);const mutable=['C:\\a'];const pending=api.writeFiles(mutable);mutable[0]='private';mutable.push('relative');await pending;assert.deepEqual(plain(params),{paths:['C:\\a']});
});
await test('完整同步SDK一次native调用；无bridge不回退旧clipboard写入',async()=>{
 const good=sync();assert.deepEqual(plain(good.api.writeFiles(paths)),ok);assert.equal(good.calls.length,1);assert.equal(good.calls[0].method,'clipboard.write_files');
 const missing=sync({bridge:false});assert.throws(()=>missing.api.writeFiles(paths),predicate('native_bridge_unavailable','not_started'));assert.equal(missing.calls.length,0);
});
await test('实际Node nativeCall一次窄方法发送且无bridge可证未开始',async()=>{
 const good=node();assert.deepEqual(plain(await good.api.writeFiles(paths)),ok);assert.equal(good.calls.length,1);assert.deepEqual(good.calls[0].params,{paths});assert.equal(good.timers.size,0);
 const missing=node({bridge:false});await assert.rejects(missing.api.writeFiles(paths),predicate('native_bridge_unavailable','not_started'));assert.equal(missing.calls.length,0);
});
await test('传输前错误not_started；发送后取消和超时unknown且不重试',async()=>{
 const pre=node({mode:'pre_error'});await assert.rejects(pre.api.writeFiles(paths),predicate('desktop_connection','not_started'));assert.equal(pre.calls.length,0);
 const cancel=node({mode:'hold'});const a=cancel.api.writeFiles(paths);await new Promise(resolve=>setImmediate(resolve));cancel.controller.abort(Object.assign(new Error('private'),{code:'cancelled'}));await assert.rejects(a,predicate('cancelled','unknown'));assert.equal(cancel.calls.length,1);
 const timeout=node({mode:'hold'});const b=timeout.api.writeFiles(paths);await new Promise(resolve=>setImmediate(resolve));[...timeout.timers.values()][0]();await assert.rejects(b,predicate('timeout','unknown'));assert.equal(timeout.calls.length,1);
});
await test('畸形回执或错误方法不得当写入成功',async()=>{
 for(const value of [null,{},true,{...ok,paths:['private']},{written:false,sequenceNumber:1},{written:true,sequenceNumber:-1},{written:true,sequenceNumber:2**32}])await assert.rejects(node({value}).api.writeFiles(paths),predicate('invalid_response','unknown'));
 await assert.rejects(node({error:{code:'clipboard_failed',message:'private',method:'wrong.method',effect_state:'not_started'}}).api.writeFiles(paths),predicate('invalid_response','unknown'));
});
await test('结构化前置拒绝与写后失败保持效果，隐藏正文权限字段',async()=>{
 for(const effect of ['not_started','unknown'])await assert.rejects(node({error:{code:'clipboard_failed',message:'private',method:'clipboard.write_files',effect_state:effect,required_permissions:['private']}}).api.writeFiles(paths),predicate('clipboard_failed',effect));
});

  assert.equal(passed,8);
});
