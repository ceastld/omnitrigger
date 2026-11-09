// 真实公共工厂/Script SDK/Node nativeCall；内存Native与socket，不进行排序近似或OS操作。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import vm from 'node:vm';
import {EventEmitter} from 'node:events';
import test from 'node:test';
const repo=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const read=path=>readFileSync(resolve(repo,path),'utf8');
const desktop=read('crates/omnitrigger-actions/runtime/desktop-api.js');
const host=read('crates/omnitrigger-actions/runtime/host.mjs');
const sdk=read('tools/js-engine-probe/src/sdk.js');
const shared=['browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js','state-api.js'].map(name=>read('crates/omnitrigger-actions/runtime/'+name)).join('\n');
const create=vm.runInThisContext('(()=>{'+desktop+';return createDesktopApi;})()');
const wire='fs.sort_logical_paths';
const fault=code=>e=>e.code===code&&e.method===wire&&e.effectState==='not_started';
const plain=v=>JSON.parse(JSON.stringify(v));

test('只转发完整字符串，保留空串/NUL/重复/相对文本；返回Native原顺序的冻结副本',()=>{
  const input=['a10','a2','','dup','dup','\0tail','OneDrive/relative'],before=input.slice();
  const native=['dup','a10','a2','','dup','\0tail','OneDrive/relative'];
  let count=0;const api=create((method,params)=>{count++;assert.equal(method,wire);assert.deepEqual(Object.keys(params),['paths']);assert.deepEqual(params.paths,before);assert(Object.isFrozen(params.paths));return native;}).fs;
  const value=api.sortLogicalPaths(input);
  assert.deepEqual(value,native);assert.notEqual(value,native);assert(Object.isFrozen(value));assert(!Object.isFrozen(input));
  native[0]='changed';input[0]='caller changed';assert.equal(value[0],'dup');assert.equal(count,1);
});
test('空数组、单元素也通过Native检查平台能力，不走本地快速返回',()=>{
  const calls=[];const api=create((method,params)=>{calls.push({method,params});return params.paths;}).fs;
  assert.deepEqual(api.sortLogicalPaths([]),[]);assert.deepEqual(api.sortLogicalPaths(['\0']),['\0']);assert.equal(calls.length,2);
});
test('异步连接等待期间调用方修改不影响已冻结快照与响应多重集',async()=>{
  let finish;const input=['old','same','same'];let params;
  const api=create((_method,p)=>{params=p;return new Promise(resolve=>{finish=resolve;});}).fs;
  const promise=api.sortLogicalPaths(input);input[0]='new';input.push('extra');
  assert.deepEqual(params.paths,['old','same','same']);finish(['same','old','same']);
  const value=await promise;assert.deepEqual(value,['same','old','same']);assert(Object.isFrozen(value));
});
test('参数个数、元素数、UTF16及未配对代理在派发前拒绝',()=>{
  let calls=0;const api=create(()=>{calls++;return [];}).fs;
  for(const args of [[],[[],'extra'],[undefined],[null],[{}],['text'],[new Uint8Array(1)],[[1]],[[new String('x')]],[[null]],[[true]],[[Symbol('x')]],[[1n]],[['\ud800']],[['\udc00']],[['x'.repeat(32768)]],[Array(1025).fill('')]])
    assert.throws(()=>api.sortLogicalPaths(...args),fault('invalid_input'));
  assert.equal(calls,0);
});
test('holes/accessor/额外属性/符号/subclass严格拒绝，验证不调用getter',()=>{
  let getters=0,calls=0;const getter=()=>{getters++;throw Error('GETTER MUST NOT RUN');};
  const accessor=['x'];Object.defineProperty(accessor,'0',{get:getter});
  const extra=['x'];Object.defineProperty(extra,'secret',{get:getter});
  const symbol=['x'];Object.defineProperty(symbol,Symbol('extra'),{get:getter});
  const toJson=['x'];Object.defineProperty(toJson,'toJSON',{get:getter});
  const hole=new Array(1);const nullProto=['x'];Object.setPrototypeOf(nullProto,null);
  class Subclass extends Array {}
  const api=create(()=>{calls++;return [];}).fs;
  for(const value of [accessor,extra,symbol,toJson,hole,nullProto,new Subclass('x')])assert.throws(()=>api.sortLogicalPaths(value),fault('invalid_input'));
  assert.equal(getters,0);assert.equal(calls,0);
});
test('1024项和单串32767 UTF16边界可用，不归一化或裁剪',()=>{
  const api=create((_method,p)=>p.paths).fs;
  assert.equal(api.sortLogicalPaths(Array(1024).fill('')).length,1024);
  assert.equal(api.sortLogicalPaths(['x'.repeat(32767)])[0].length,32767);
  assert.deepEqual(api.sortLogicalPaths([' ','relative/../x','C:\\a\0suffix','A','a']),[' ','relative/../x','C:\\a\0suffix','A','a']);
});
test('64KiB按JSON的UTF8字节计算，含ASCII/中文/代理对/NUL/引号转义精确边界',()=>{
  let calls=0;const api=create((_method,p)=>{calls++;return p.paths;}).fs;
  const exact=[['x'.repeat(32767),'y'.repeat(32752)],['界'.repeat(21840)+'aa'],['😀'.repeat(16380)+'aa'],['\0'.repeat(10920)+'aa'],['"'.repeat(32761)]];
  for(const paths of exact){assert.equal(Buffer.byteLength(JSON.stringify({paths})),65536);assert.deepEqual(api.sortLogicalPaths(paths),paths);}
  const before=calls;
  for(const paths of [['x'.repeat(32767),'y'.repeat(32753)],['界'.repeat(21840)+'aaa'],['😀'.repeat(16380)+'aaa'],['\0'.repeat(10920)+'aaa'],['"'.repeat(32762)]]) {
    assert(Buffer.byteLength(JSON.stringify({paths}))>65536);assert.throws(()=>api.sortLogicalPaths(paths),fault('invalid_input'));
  }
  assert.equal(calls,before);
});
test('响应长度/完整内容/重复计数/普通数组校验，坏回执不执行getter',async()=>{
  let getters=0;const accessor=['a','b'];Object.defineProperty(accessor,'0',{get(){getters++;throw Error('GETTER MUST NOT RUN');}});
  const extra=['a','b'];Object.defineProperty(extra,'toJSON',{get(){getters++;throw Error('GETTER MUST NOT RUN');}});
  for(const value of [null,{}, {paths:['a','b']},'ab',[],['a'],['a','b','b'],['a','a'],['a','B'],new Array(2),accessor,extra]) {
    assert.throws(()=>create(()=>value).fs.sortLogicalPaths(['a','b']),fault('fs_error'));
    await assert.rejects(create(()=>Promise.resolve(value)).fs.sortLogicalPaths(['a','b']),fault('fs_error'));
  }
  assert.equal(getters,0);
});
test('Native错误保留只读类别、调用上限与method，不重试、不回显输入',async()=>{
  for(const code of ['invalid_input','fs_error','fs_unavailable','fs_busy','cancelled','timeout','desktop_call_limit','unexpected']) {
    let calls=0;const cause=Object.assign(Error('PRIVATE input'),{code,effectState:'unknown',method:'wrong'});
    const expected=code==='unexpected'?'fs_error':code;
    assert.throws(()=>create(()=>{calls++;throw cause;}).fs.sortLogicalPaths(['private']),fault(expected));
    await assert.rejects(create(()=>{calls++;return Promise.reject(cause);}).fs.sortLogicalPaths(['private']),fault(expected));assert.equal(calls,2);
  }
});
function scriptBoot(context) {
  const calls=[];const scope={ctx:{input:{},__nativeContext:context,__localDesktopAllowed:false},__hostCall:raw=>{
    const request=JSON.parse(raw);calls.push(request);return JSON.stringify({ok:true,value:request.kind==='delay'?null:request.params.paths});
  }};const sandbox=vm.createContext(scope);
  new vm.Script(desktop+'\n'+shared+'\n'+sdk).runInContext(sandbox,{timeout:1000});
  return {calls,run:source=>vm.runInContext(source,sandbox,{timeout:1000})};
}
test('真实Script SDK允许无foreground但有桥接，保持同步JSON/一次宿主计数',()=>{
  const s=scriptBoot({connection:{port:1,token:'a'.repeat(64)},native_context_available:false});
  const value=s.run('ctx.fs.sortLogicalPaths(["relative","", "relative"])');
  assert.deepEqual(plain(value),['relative','','relative']);assert(Object.isFrozen(value));assert.equal(typeof value.then,'undefined');
  assert.equal(s.calls.length,1);assert.equal(s.calls[0].kind,'native');assert.equal(s.calls[0].method,wire);assert.deepEqual(s.calls[0].params,{paths:['relative','','relative']});
});
test('Script缺桥不回退本地；32个同步宿主名额计入排序，超额不派发',()=>{
  for(const context of [null,{native_context_available:false},{error:{code:'native_bridge_unavailable',effect_state:'not_started'}}]){
    const s=scriptBoot(context);assert.throws(()=>s.run('ctx.fs.sortLogicalPaths([])'),fault('fs_unavailable'));assert.equal(s.calls.length,0);
  }
  const s=scriptBoot({connection:{port:1,token:'a'.repeat(64)},native_context_available:false});
  s.run('for(let i=0;i<32;i++)ctx.fs.sortLogicalPaths([])');assert.equal(s.calls.length,32);
  assert.throws(()=>s.run('ctx.fs.sortLogicalPaths([])'),fault('desktop_call_limit'));assert.equal(s.calls.length,32);
});
const nativeBody=host.slice(host.indexOf('function nativeCall('),host.indexOf('const desktopApis ='));
const nativeMethodsStart=host.indexOf('const nativeMethods =');
const nativeMethods=host.slice(nativeMethodsStart,host.indexOf(']);',nativeMethodsStart)+3);
const asyncApiStart=host.indexOf('const asyncApi =');const asyncApi=host.slice(asyncApiStart,host.indexOf('\n',asyncApiStart));
function nodeBoot() {
  const sockets=[],timers=[];class Socket extends EventEmitter {
    setEncoding(value){this.encoding=value;}setTimeout(ms,handler){this.timeout=ms;this.expired=handler;}
    write(text){this.request=JSON.parse(text);}destroy(){this.destroyed=true;}
  }
  const controller=new AbortController();
  const scope={Buffer,TextDecoder,controller,nativeSockets:new Set(),nativeId:0,nativePending:undefined,nativePendingMethod:undefined,
    request:{native:{port:1,token:'synthetic'},native_context_available:false},fault:(code,message)=>Object.assign(Error(message),{code}),
    checkCancelled:()=>{if(controller.signal.aborted)throw controller.signal.reason;},
    createConnection:()=>{const socket=new Socket();sockets.push(socket);return socket;},
    setTimeout:(callback,ms)=>{const timer={callback,ms};timers.push(timer);return timer;},clearTimeout:timer=>{if(timer)timer.cleared=true;}};
  const sandbox=vm.createContext(scope);
  vm.runInContext(desktop+'\n'+nativeMethods+'\n'+nativeBody+'\n'+asyncApi+'\nconst api=asyncApi(createDesktopApi(nativeCall).fs);',sandbox);
  const run=source=>vm.runInContext(source,sandbox,{timeout:1000});
  const start=paths=>{const promise=run('api.sortLogicalPaths('+JSON.stringify(paths)+')');const socket=sockets.at(-1);socket?.emit('connect');return {promise,socket};};
  const send=(socket,value)=>socket.emit('data',Buffer.from(JSON.stringify({id:socket.request.id,ok:true,value})+'\n'));
  return {scope,controller,sockets,timers,run,start,send};
}
test('真实Node nativeMethods/asyncApi/无foreground路径，纯读5秒总限时及冻结结果',async()=>{
  const n=nodeBoot(),{promise,socket}=n.start(['file10','file2']);assert.equal(typeof promise.then,'function');
  assert.equal(socket.encoding,undefined);assert.equal(socket.timeout,5000);assert.equal(n.timers[0].ms,5000);
  assert.equal(socket.request.method,wire);assert.deepEqual(socket.request.params,{paths:['file10','file2']});
  n.send(socket,['file2','file10']);const value=await promise;assert.deepEqual(plain(value),['file2','file10']);assert(Object.isFrozen(value));assert(socket.destroyed);
  assert.equal(n.scope.request.native_context_available,false);assert.equal(n.scope.nativeId,1);
});
test('Node缺桥/参数错误/busy/128次上限/取消均在socket前拒绝',async()=>{
  const missing=nodeBoot();delete missing.scope.request.native;await assert.rejects(missing.start([]).promise,fault('fs_unavailable'));assert.equal(missing.sockets.length,0);
  const invalid=nodeBoot();await assert.rejects(invalid.run('api.sortLogicalPaths()'),fault('invalid_input'));assert.equal(invalid.sockets.length,0);
  const busy=nodeBoot();busy.scope.nativePending=Promise.resolve();await assert.rejects(busy.start([]).promise,fault('fs_busy'));assert.equal(busy.sockets.length,0);
  const budget=nodeBoot();budget.scope.nativeId=128;await assert.rejects(budget.start([]).promise,fault('desktop_call_limit'));assert.equal(budget.sockets.length,0);
  const cancelled=nodeBoot();cancelled.controller.abort(Object.assign(Error('stop'),{code:'cancelled'}));await assert.rejects(cancelled.start([]).promise,fault('cancelled'));assert.equal(cancelled.sockets.length,0);
});
test('Node只读超时/传输错误和非Windows fs_unavailable不被当作排序成功',async()=>{
  for(const kind of ['timeout','socket','platform']){
    const n=nodeBoot(),{promise,socket}=n.start(['x']);
    if(kind==='timeout')n.timers[0].callback();
    else if(kind==='socket')socket.emit('error',Error('PRIVATE'));
    else socket.emit('data',Buffer.from(JSON.stringify({id:socket.request.id,ok:false,error:{code:'fs_unavailable',message:'PRIVATE',method:wire,effect_state:'not_started'}})+'\n'));
    await assert.rejects(promise,fault(kind==='timeout'?'timeout':kind==='platform'?'fs_unavailable':'fs_error'));assert.equal(n.sockets.length,1);assert(socket.destroyed);
  }
});
test('Node坏UTF8字节FF不能替换成输入U+FFFD绕过多重集验证',async()=>{
  const replacement='\ufffd';assert.equal(replacement.charCodeAt(0),0xfffd);
  const n=nodeBoot(),{promise,socket}=n.start([replacement]);
  const prefix=Buffer.from('{"id":'+socket.request.id+',"ok":true,"value":["');
  socket.emit('data',Buffer.concat([prefix,Buffer.from([0xff]),Buffer.from('"]}\n')]));
  await assert.rejects(promise,fault('fs_error'));assert(socket.destroyed);assert.equal(n.sockets.length,1);
});
test('Node严格UTF8允许多字节跨chunk，不做替换；严格JSON envelope与错误method检查',async()=>{
  const n=nodeBoot(),first=n.start(['😀']);
  const bytes=Buffer.from(JSON.stringify({id:first.socket.request.id,ok:true,value:['😀']})+'\n');const at=bytes.indexOf(Buffer.from('😀'));
  first.socket.emit('data',bytes.subarray(0,at+1));first.socket.emit('data',bytes.subarray(at+1));assert.deepEqual(plain(await first.promise),['😀']);
  for(const response of [{ok:true,value:['x'],extra:1},{ok:false,error:{code:'fs_unavailable',message:'x',method:'wrong',effect_state:'not_started'}}]){
    const f=nodeBoot(),{promise,socket}=f.start(['x']);socket.emit('data',Buffer.from(JSON.stringify({id:socket.request.id,...response})+'\n'));
    await assert.rejects(promise,fault('fs_error'));
  }
});

test('数组/非Promise回执的then getter不执行，也不接受伪thenable',()=>{
  let getters=0,thenCalls=0;
  const array=['x'];Object.defineProperty(array,'then',{get(){getters++;throw Error('GETTER MUST NOT RUN');}});
  const object={};Object.defineProperty(object,'then',{get(){getters++;throw Error('GETTER MUST NOT RUN');}});
  for(const value of [array,object,{then(){thenCalls++;}}])assert.throws(()=>create(()=>value).fs.sortLogicalPaths(['x']),fault('fs_error'));
  assert.equal(getters,0);assert.equal(thenCalls,0);
});

test('排序和已有同步宿主调用共享32名额，不另开计数池',()=>{
  const s=scriptBoot({connection:{port:1,token:'a'.repeat(64)},native_context_available:false});
  s.run('ctx.delay(0);for(let i=0;i<31;i++)ctx.fs.sortLogicalPaths([])');
  assert.equal(s.calls.length,32);assert.equal(s.calls[0].kind,'delay');
  assert.throws(()=>s.run('ctx.fs.sortLogicalPaths([])'),fault('desktop_call_limit'));assert.equal(s.calls.length,32);
});
test('Node遗留未await的排序请求按现有fs纯读收尾规则拒绝',()=>{
  const match=host.match(/if \(nativePendingMethod === 'fs\.stat'.*?\n/);
  assert(match);assert(match[0].includes(wire));
  assert.throws(()=>vm.runInNewContext(match[0],{nativePendingMethod:wire,fault:(code,message)=>Object.assign(Error(message),{code})}),fault('fs_error'));
});

test('复用大字符串的1024项数组逐项超限即拒绝，0 Native且不构造完整JSON',()=>{
  const text='x'.repeat(32767),paths=Array(1024).fill(text);
  const stringify=JSON.stringify;let strings=0,calls=0,largest=0;
  const api=create(()=>{calls++;return [];}).fs;
  try {
    JSON.stringify=(value,...args)=>{
      assert.equal(typeof value,'string','不得构造整个{paths}或数组的JSON');
      strings++;largest=Math.max(largest,value.length);return stringify(value,...args);
    };
    assert.throws(()=>api.sortLogicalPaths(paths),fault('invalid_input'));
  } finally { JSON.stringify=stringify; }
  assert.equal(calls,0);assert(strings>0&&strings<=2,'预算应在头两项内停止，不遍历1024项');assert(largest<=32767);
});
