import fs from 'node:fs';
import vm from 'node:vm';
// 测试真实共享工厂、同步 SDK 及 Node nativeCall；桥接、socket 和 timer 全为内存替身。
import assert from 'node:assert/strict';
import {readFileSync,lstatSync} from 'node:fs';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {runInNewContext} from 'node:vm';
import {EventEmitter} from 'node:events';
import test from 'node:test';

const own=dirname(fileURLToPath(import.meta.url));
const repo=resolve(own,'..');
// 候选回放检查可指定根；正式 CI 默认仓库根。不访问测试路径。
const sourceRoot=process.env.OMNITRIGGER_DIRECTORY_TEST_SOURCE_ROOT ?? repo;
function read(root,path) {
  const file=resolve(root,path);
  for(let part=file;;part=dirname(part)) {
    assert(!lstatSync(part).isSymbolicLink());
    const next=dirname(part);if(next===part)break;
  }
  return readFileSync(file,'utf8');
}
const desktop=read(sourceRoot,'crates/omnitrigger-actions/runtime/desktop-api.js');
const host=read(sourceRoot,'crates/omnitrigger-actions/runtime/host.mjs');
const sdk=read(sourceRoot,'tools/js-engine-probe/src/sdk.js');
const sharedRoot=process.env.OMNITRIGGER_DIRECTORY_TEST_SHARED_ROOT ?? sourceRoot;
const shared=['browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js'].map(name=>read(sharedRoot,'crates/omnitrigger-actions/runtime/'+name)).join('\n');
const createApi=runInNewContext(desktop+';createDesktopApi');
const path='X:/synthetic-only/';
const wire='fs.read_directory';
const plain=value=>JSON.parse(JSON.stringify(value));
const result=(names=['a.txt'])=>({entries:names.map(name=>({name,path:path+name,kind:'file',reason:null})),complete:true});
const fault=code=>error=>error.code===code&&error.method===wire&&error.effectState==='not_started'&&!error.message.includes('PRIVATE');

test('目录 API 逐字传目录、默认及边界上限，Node Promise 和同步值语义一致',async()=>{
  for(const async of [false,true]) {
    const calls=[];
    const fs=createApi((method,params)=>{calls.push([method,plain(params)]);return async?Promise.resolve(result()):result();}).fs;
    const pending=fs.readDirectory(path);
    assert.equal(typeof pending.then==='function',async);
    const value=await pending;
    assert.deepEqual(plain(value),result());
    assert(Object.isFrozen(value)&&Object.isFrozen(value.entries)&&Object.isFrozen(value.entries[0]));
    assert.deepEqual(calls.shift(),[wire,{path,max_entries:128}]);
    await fs.readDirectory(path,{maxEntries:1}); await fs.readDirectory(path,{maxEntries:1024});
    assert.deepEqual(calls.map(v=>v[1].max_entries),[1,1024]);
  }
});
test('非法路径和选项在宿主前拒绝；不进行归一化或默认选目录',()=>{
  let calls=0;const fs=createApi(()=>{calls++;return result();}).fs;
  for(const value of [undefined,null,[],{},'', 'relative','C:relative','\\\\host\\share','\\\\?\\C:\\x','C:/a/../b','C:/a//b','C:/NUL.txt','C:/a.','C:/a:stream','C:/a\ud800','C:/a\0'])
    assert.throws(()=>fs.readDirectory(value),fault('invalid_input'));
  for(const options of [null,[],1,true,{limit:1},{maxEntries:0},{maxEntries:1025},{maxEntries:1.5},{maxEntries:'1'},{maxEntries:NaN},{[Symbol('extra')]:1}])
    assert.throws(()=>fs.readDirectory(path,options),fault('invalid_input'));
  assert.throws(()=>fs.readDirectory(path,{},{}),fault('invalid_input'));
  assert.throws(()=>fs.readDirectory(path,{get maxEntries(){throw Error('PRIVATE');}}),fault('invalid_input'));
  for(const value of ['C:/OneDrive/a','C:/safe/Dropbox/a','C:/iCloudDocs/a']) assert.throws(()=>fs.readDirectory(value),fault('path_unavailable'));
  assert.equal(calls,0);
});
test('保留不可用直接子项，不把未知/超量/不完整回执当成功',async()=>{
  const value={entries:[{name:'link',path:path+'link',kind:'unavailable',reason:'reparse_point'},
    {name:'OneDrive-work',path:path+'OneDrive-work',kind:'unavailable',reason:'cloud_path'},
    {name:'name.',path:path+'name.',kind:'unavailable',reason:'unsupported_name'}],complete:true};
  assert.deepEqual(plain(createApi(()=>value).fs.readDirectory(path)),value);
  const malformed=[null,[],{}, {...result(),complete:false},{...result(),extra:1},result(['a','a']),
    {entries:[{...result().entries[0],path:'X:/other/a.txt'}],complete:true},
    {entries:[{...result().entries[0],name:'../x'}],complete:true},
    {entries:[{...result().entries[0],kind:'unavailable',reason:null}],complete:true},
    {entries:[{...result().entries[0],reason:'reparse_point'}],complete:true},
    {entries:[{name:'OneDrive',path:path+'OneDrive',kind:'directory',reason:null}],complete:true},
    result(['a','b']),result(['中'.repeat(12000)])];
  for(const value of malformed) for(const async of [false,true]) {
    const fs=createApi(()=>async?Promise.resolve(value):value).fs;
    if(async)await assert.rejects(fs.readDirectory(path,{maxEntries:1}),fault('fs_error'));
    else assert.throws(()=>fs.readDirectory(path,{maxEntries:1}),fault('fs_error'));
  }
});
test('所有已知平台错误保留类别并固定错误正文，未知错误归为 fs_error',async()=>{
  for(const code of ['invalid_input','path_not_local','path_unavailable','path_missing','not_directory','directory_too_large','directory_changed','access_denied','fs_error','fs_unavailable','fs_busy','cancelled','timeout','unexpected']) {
    const error=Object.assign(Error('PRIVATE'),{code,method:'wrong',effectState:'unknown'});
    const expected=code==='unexpected'?'fs_error':code;
    assert.throws(()=>createApi(()=>{throw error;}).fs.readDirectory(path),fault(expected));
    await assert.rejects(createApi(()=>Promise.reject(error)).fs.readDirectory(path),fault(expected));
  }
});
test('真实轻量 SDK 走 native 请求，缺少宿主明确 fs_unavailable，不借用 readText',()=>{
  for(const connected of [false,true]) {
    const calls=[];const scope={ctx:connected?{__nativeContext:{connection:{port:1,token:'c'.repeat(64)}}}:{},__hostCall:text=>{
      const frame=JSON.parse(text);calls.push(frame);return JSON.stringify({ok:true,value:result()});
    }};
    runInNewContext(desktop+shared+sdk,scope);
    if(connected) {
      assert.deepEqual(plain(scope.ctx.fs.readDirectory(path)),result());
      assert.equal(calls[0].method,wire);assert.deepEqual(calls[0].params,{path,max_entries:128});
      assert.equal(typeof scope.ctx.fs.readText,'function');assert.equal(scope.ctx.__nativeContext,undefined);
    }else {assert.throws(()=>scope.ctx.fs.readDirectory(path),fault('fs_unavailable'));assert.equal(calls.length,0);}
  }
});
test('真实 Node asyncApi 包装把同步词法错误转为 Promise 拒绝',async()=>{
  const declaration=host.match(/const asyncApi = ([^\n]+);/)?.[1];assert(declaration);
  assert(host.includes('const fs = asyncApi(desktopApis.fs);'));
  const asyncApi=runInNewContext(declaration);
  const fs=asyncApi(createApi(()=>{throw Error('不应到宿主');}).fs);
  const pending=fs.readDirectory('relative');
  assert.equal(typeof pending.then,'function');await assert.rejects(pending,fault('invalid_input'));
});

function harness() {
  const sockets=[],timers=[];
  class Socket extends EventEmitter {
    setEncoding(value){this.encoding=value;}
    setTimeout(ms,handler){this.timeout=ms;this.expired=handler;}
    write(text){this.request=JSON.parse(text);}
    destroy(){this.destroyed=true;}
  }
  const controller=new AbortController();
  const methods=host.match(/const nativeMethods = (new Set\([\s\S]*?\));/)?.[1];assert(methods);
  const scope={Buffer,TextDecoder,Promise,Set,controller,nativeMethods:runInNewContext(methods),nativeSockets:new Set(),
    nativeId:0,nativePending:undefined,nativePendingMethod:undefined,request:{native:{port:1,token:'synthetic'}},
    fault:(code,message)=>Object.assign(Error(message),{code}),checkCancelled:()=>{if(controller.signal.aborted)throw controller.signal.reason;},
    createConnection:()=>{const socket=new Socket();sockets.push(socket);return socket;},
    setTimeout:(callback,ms)=>{const timer={callback,ms};timers.push(timer);return timer;},clearTimeout:timer=>{if(timer)timer.cleared=true;},
  };
  const body=host.slice(host.indexOf('function nativeCall('),host.indexOf('const desktopApis ='));
  const call=runInNewContext(body+'\nnativeCall;',scope);
  const fs=createApi((method,params)=>call(method,params)).fs;
  const start=()=>{const pending=fs.readDirectory(path);const socket=sockets.at(-1);socket?.emit('connect');return {pending,socket};};
  const send=(socket,reply)=>socket.emit('data',JSON.stringify({id:socket.request.id,...reply})+'\n');
  return {scope,sockets,timers,controller,fs,start,send};
}
test('真实 Node nativeCall 白名单、精确请求与成功回执，单调用忙碌明确 fs_busy',async()=>{
  const h=harness(),{pending,socket}=h.start();
  assert.deepEqual(socket.request.params,{path,max_entries:128});assert.equal(socket.request.method,wire);
  assert.equal(socket.timeout,5000);assert.equal(h.timers[0].ms,5000);
  await assert.rejects(h.fs.readDirectory(path),fault('fs_busy'));assert.equal(h.sockets.length,1);
  h.send(socket,{ok:true,value:result()});assert.deepEqual(plain(await pending),result());assert(h.timers[0].cleared);
});
test('真实 Node 桥接缺失、连接错误、取消和时限不返回部分子项且不重试',async()=>{
  const absent=harness();delete absent.scope.request.native;
  await assert.rejects(absent.fs.readDirectory(path),fault('fs_unavailable'));assert.equal(absent.sockets.length,0);
  for(const mode of ['cancel','timeout','close','error']) {
    const h=harness(),{pending,socket}=h.start();socket.emit('data','{"id":1,');
    if(mode==='cancel')h.controller.abort(Object.assign(Error('PRIVATE'),{code:'cancelled'}));
    if(mode==='timeout')h.timers[0].callback();
    if(mode==='close')socket.emit('close');
    if(mode==='error')socket.emit('error',Error('PRIVATE'));
    await assert.rejects(pending,fault(mode==='cancel'?'cancelled':mode==='timeout'?'timeout':'fs_error'));
    assert.equal(h.sockets.length,1);assert(socket.destroyed);
  }
});
test('真实 Node 桥接拒绝错身份/额外字段/错方法，平台错误无路径或权限回显',async()=>{
  for(const reply of [{ok:true,value:result(),extra:true},{ok:false,error:{code:'access_denied',message:'PRIVATE',method:'other'}}]) {
    const h=harness(),{pending,socket}=h.start();h.send(socket,reply);await assert.rejects(pending,fault('fs_error'));
  }
  const h=harness(),{pending,socket}=h.start();h.send(socket,{ok:false,error:{code:'directory_too_large',message:'PRIVATE',required_permissions:['invented']}});
  await assert.rejects(pending,error=>fault('directory_too_large')(error)&&!Object.hasOwn(error,'requiredPermissions'));
});

// 目录元数据既有七组共享 SDK 合同，不访问测试路径。
test('目录元数据 SDK 端口合同', async t => {
const here=new URL('../',import.meta.url);
const relative='crates/omnitrigger-actions/runtime/desktop-api.js';
const after=fs.readFileSync(new URL(relative,here),'utf8');
const plain=value=>JSON.parse(JSON.stringify(value));
const path='C:\\synthetic';
const item=name=>({name,path:path+'\\'+name,kind:'file',reason:null});
const metadata=(ordinal,tick='638975574001234567')=>({ordinal,attributes:0,
 creationTimeUtc100ns:'134064342001234567',lastAccessTimeUtc100ns:'134064342001234567',lastWriteTimeUtc100ns:'134064342001234567',
 creationTimeLocalTicks100ns:tick,lastAccessTimeLocalTicks100ns:tick,lastWriteTimeLocalTicks100ns:tick,timeZoneKey:'Pacific Standard Time'});
const reply=entries=>({entries,complete:true});
function api(source,value,asynchronous=false){
 const calls=[];const c=vm.createContext({invoke:(method,params)=>{calls.push({method,params});return asynchronous?Promise.resolve(value):value;}});
 vm.runInContext(source+'\nglobalThis.api=createDesktopApi(invoke);',c);
 return {fs:c.api.fs,calls};
}

await t.test('默认参数、请求和返回形状保持既有契约',async()=>{
 for(const asynchronous of [false,true]){
  const b=api(after,reply([item('a')]),asynchronous);
  assert.deepEqual(plain(await b.fs.readDirectory(path)),plain(reply([item('a')])));
  assert.deepEqual(plain(b.calls),[{method:'fs.read_directory',params:{path,max_entries:128}}]);
 }
});

await t.test('同步和异步端口保留原枚举顺序、序号及100ns字符串，不按毫秒重算',async()=>{
 for(const asynchronous of [false,true]){
  const b=api(after,reply([{...item('z'),metadata:metadata(0)},{...item('a'),metadata:metadata(1,'638975574001234568')}]),asynchronous);
  const output=await b.fs.readDirectory(path,{maxEntries:4,order:'native',metadata:true,entryType:'file',searchPattern:'*.xls',matchMode:'windows-find'});
  assert.deepEqual(plain(output.entries.map(x=>x.name)),['z','a']);
  assert.equal(BigInt(output.entries[1].metadata.lastWriteTimeLocalTicks100ns)-BigInt(output.entries[0].metadata.lastWriteTimeLocalTicks100ns),1n);
  assert(Object.isFrozen(output.entries[0].metadata));
  assert.deepEqual(plain(b.calls[0].params),{path,max_entries:4,order:'native',metadata:true,entry_type:'file',search_pattern:'*.xls',match_mode:'windows-find'});
 }
});

await t.test('名称投影可以保留原序号，属性原样保留供Hidden/System位检查',()=>{
 const a={...metadata(1),attributes:6},b=api(after,reply([{...item('a'),metadata:a},{...item('z'),metadata:metadata(0)}]));
 const result=b.fs.readDirectory(path,{metadata:true});
 assert.equal(result.entries[0].metadata.ordinal,1);assert.equal(result.entries[0].metadata.attributes&6,6);
});

await t.test('坏选项、未绑定匹配口径与越目录模式在零invoke前拒绝',()=>{
 const b=api(after,reply([]));
 for(const options of [{maxEntries:null},{metadata:null},{metadata:1},{order:'other'},{entryType:'other'},
  {searchPattern:'*.xls'},{matchMode:'windows-find'},{searchPattern:'*',matchMode:'dotnet-managed'},
  {searchPattern:'sub\\*.txt',matchMode:'windows-find'},{searchPattern:'',matchMode:'windows-find'},
  {searchPattern:'..',matchMode:'windows-find'},{searchPattern:'a\0b',matchMode:'windows-find'},{searchPattern:'*',matchMode:'windows-find',extra:true}]){
  assert.throws(()=>b.fs.readDirectory(path,options),error=>error.code==='invalid_input'&&error.effectState==='not_started');
 }
 assert.equal(b.calls.length,0);
});

await t.test('参数在invoke前固定，不受调用方随后变更影响',async()=>{
 const b=api(after,reply([]),true),options={metadata:true,order:'native',searchPattern:'*',matchMode:'windows-find'};
 const pending=b.fs.readDirectory(path,options);options.order='name';options.searchPattern='*.wrong';
 await pending;assert.equal(b.calls[0].params.order,'native');assert.equal(b.calls[0].params.search_pattern,'*');
});

await t.test('元数据缺失、坏精度/溢出/序号重复、额外字段拒绝，不掩盖默认契约',()=>{
 const bad=[undefined,{...metadata(0),lastWriteTimeLocalTicks100ns:123},
  {...metadata(0),lastWriteTimeLocalTicks100ns:'0638975574001234567'},
  {...metadata(0),lastWriteTimeUtc100ns:'2650467744000000000'},
  {...metadata(0),lastWriteTimeLocalTicks100ns:'3155378976000000000'},
  {...metadata(0),attributes:2**32},{...metadata(0),ordinal:1},{...metadata(0),extra:true}];
 for(const m of bad){const b=api(after,reply([{...item('a'),...(m?{metadata:m}:{})}]));assert.throws(()=>b.fs.readDirectory(path,{metadata:true}),error=>error.code==='fs_error');}
 const duplicate=api(after,reply([{...item('a'),metadata:metadata(0)},{...item('b'),metadata:metadata(0)}]));
 assert.throws(()=>duplicate.fs.readDirectory(path,{metadata:true}),error=>error.code==='fs_error');
 const defaultReply=api(after,reply([{...item('a'),metadata:metadata(0)}]));
 assert.throws(()=>defaultReply.fs.readDirectory(path),error=>error.code==='fs_error');
});

await t.test('通配符原文只传系统端口，不执行JS正则匹配或改写为全匹配',()=>{
 for(const pattern of ['*.xls','a?.txt','*.*','.']){
  const b=api(after,reply([]));b.fs.readDirectory(path,{searchPattern:pattern,matchMode:'windows-find'});
  assert.equal(b.calls[0].params.search_pattern,pattern);assert.equal(b.calls.length,1);
 }
});


});
