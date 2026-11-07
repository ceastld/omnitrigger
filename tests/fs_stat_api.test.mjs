import assert from 'node:assert/strict';
import {readFileSync, writeFileSync, mkdirSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {resolve, join, dirname, relative, isAbsolute} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createServer} from 'node:net';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {runInNewContext} from 'node:vm';
import test from 'node:test';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const runtime = resolve(repo, 'crates/omnitrigger-actions/runtime');
const desktop = readFileSync(join(runtime,'desktop-api.js'),'utf8');
const browser = readFileSync(join(runtime,'browser-api.js'),'utf8');
const history = readFileSync(join(runtime,'clipboard-history-api.js'),'utf8')
  + readFileSync(join(runtime,'process-capture-api.js'),'utf8')
  + readFileSync(join(runtime,'actions-api.js'),'utf8');
const host = readFileSync(join(runtime,'host.mjs'),'utf8');
const sdk = readFileSync(join(repo,'tools/js-engine-probe/src/sdk.js'),'utf8');
const createApi = runInNewContext(desktop+';createDesktopApi');
const path = 'X:\\test-only-path-marker\\file.txt'; // 只传给内存接收端，不查询此路径。
const token = 'c'.repeat(64);
const plain = value => JSON.parse(JSON.stringify(value));
const fault = code => error => error.code === code && error.method === 'fs.stat' && error.effectState === 'not_started'
  && !String(error.message).includes('test-only-path-marker') && !String(error.message).includes(token);

function invalidPaths() {
  const values = [undefined,null,true,1,{},[],new String(path),'','C','C:','C:relative','relative','/root','\\root',
    '\\\\server\\share','//server/share','\\\\?\\C:\\safe','\\\\.\\C:\\safe','\\??\\C:\\safe',
    '1:\\safe','é:\\safe','Ｃ:\\safe','C::\\safe','C:\\\\','C:\\a\\\\b','C:/a//b','C:\\a/\\b',
    'C:\\a\\\\','C:\\.','C:\\..','C:\\a\\.\\b','C:\\a\\..\\b','C:\\a.','C:\\a ',
    'C:\\a.\\b','C:\\a \\b','C:\\a:stream','C:\\a:b:c','C:\\a*','C:\\a?','C:\\a<',
    'C:\\a>','C:\\a"','C:\\a|','C:\\test-only-path-marker\\\ud800','C:\\\udc00','C:\\a\ud800b',
    'C:\\'+'a'.repeat(32765),'C:\\'+'😀'.repeat(16383),'C:\\OneDrive\\..\\x','C:/Dropbox/a.','C:/iCloud//x'];
  for (const name of ['CON','PRN','AUX','NUL',...Array.from({length:9},(_,i)=>`COM${i+1}`),...Array.from({length:9},(_,i)=>`LPT${i+1}`),
    ...['¹','²','³'].flatMap(number=>[`COM${number}`,`LPT${number}`])]) {
    values.push(`C:\\${name}`,`C:\\${name.toLowerCase()}.txt`,`C:\\${name} .txt`,`C:\\a\\${name}.x\\child`);
  }
  for (let unit=0;unit<=0x9f;unit++) if (unit<0x20||unit>=0x7f) values.push(`C:\\a${String.fromCharCode(unit)}b`);
  return values;
}

test('词法校验完整拒绝类型、路径命名空间、组件、控制字符和 UTF-16 越界', () => {
  let calls=0;
  const fs=createApi(()=>{calls++;return {kind:'file'};}).fs;
  for (const value of invalidPaths()) assert.throws(()=>fs.stat(value),fault('invalid_input'));
  assert.throws(()=>fs.stat(),fault('invalid_input'));
  assert.throws(()=>fs.stat(path,{}),fault('invalid_input'));
  assert.equal(calls,0,'非法路径不能进入本机宿主');
});

test('云盘组件为 path_unavailable，盘符根、单尾分隔符和边界原文传递', () => {
  const calls=[];
  const fs=createApi((method,params)=>{calls.push([method,plain(params)]);return {kind:'other'};}).fs;
  for(const component of ['OneDrive','ONEDRIVE-work','onedrive.txt','iCloud','ICLOUD~1','Dropbox','dRoPbOx']) {
    for(const value of [`C:\\${component}\\file`,`c:/safe/${component}/`]) assert.throws(()=>fs.stat(value),fault('path_unavailable'));
  }
  assert.equal(calls.length,0);
  const valid=['C:\\','z:/','C:\\a\\','c:/a/','C:\\a/b\\c','C:\\ leading space','C:\\.hidden',
    'C:\\COM0','C:\\COM10.txt','C:\\COM⁴.txt','C:\\LPT0','C:\\CONSOLE','C:\\Dropbox.txt','C:\\Dropbox2','C:\\myOneDrive',
    'C:\\%ENV% & literal','C:\\中文😀','C:\\'+'a'.repeat(32764),'C:\\'+'😀'.repeat(16382)];
  for(const value of valid) {
    assert.deepEqual(fs.stat(value),{kind:'other'});
    assert.deepEqual(calls.pop(),['fs.stat',{path:value}]);
  }
});

test('平台错误逐类保留，路径与凭据正文移除；未知结果不能当作 missing', async () => {
  for(const code of ['invalid_input','path_not_local','path_unavailable','access_denied','fs_error','fs_unavailable','fs_busy','cancelled','timeout']) {
    const error=Object.assign(new Error(path+' '+token),{code,effectState:'unknown',method:'wrong.method'});
    assert.throws(()=>createApi(()=>{throw error;}).fs.stat(path),fault(code));
    await assert.rejects(createApi(()=>Promise.reject(error)).fs.stat(path),fault(code));
  }
  for(const value of [null,[],{},true,'missing',{kind:'unknown'},{kind:'file',path},{kind:'file',[Symbol('extra')]:true}]) {
    assert.throws(()=>createApi(()=>value).fs.stat(path),fault('fs_error'));
    await assert.rejects(createApi(()=>Promise.resolve(value)).fs.stat(path),fault('fs_error'));
  }
  for(const kind of ['file','directory','missing','other']) assert.deepEqual(createApi(()=>({kind})).fs.stat(path),{kind});
});

function light(native, reply={ok:true,value:{kind:'directory'}}) {
  const calls=[];
  const context={ctx:native===undefined?{}:{__nativeContext:native},__hostCall:serialized=>{
    calls.push(JSON.parse(serialized));return JSON.stringify(reply);
  }};
  runInNewContext(desktop+browser+history+sdk,context);
  return {ctx:context.ctx,calls};
}

test('轻量同步复用共享 fs 对象，保留已有读写方法且不暴露桥接配置', () => {
  const fixture=light({connection:{port:1234,token}});
  const result=fixture.ctx.fs.stat(path);
  assert.deepEqual(plain(result),{kind:'directory'});
  assert.equal(typeof result.then,'undefined');
  assert.equal(typeof fixture.ctx.fs.readText,'function');
  assert.equal(typeof fixture.ctx.fs.writeText,'function');
  assert.equal(fixture.calls[0].method,'fs.stat');
  assert.deepEqual(fixture.calls[0].params,{path});
  assert.equal(fixture.ctx.__nativeContext,undefined);
  assert(!JSON.stringify(fixture.ctx).includes(token));
  const isolated=light();
  for(const value of invalidPaths()) assert.throws(()=>isolated.ctx.fs.stat(value),fault('invalid_input'));
  assert.throws(()=>isolated.ctx.fs.stat(path),fault('fs_unavailable'));
  const unavailable=light({error:{code:'desktop_unavailable',message:path+' '+token,effect_state:'unknown'}});
  assert.throws(()=>unavailable.ctx.fs.stat(path),fault('fs_unavailable'));
  assert.equal(isolated.calls.length+unavailable.calls.length,0);
});

test('轻量错误仍为只读 not_started，不回显路径或底层错误正文', () => {
  for(const code of ['path_not_local','path_unavailable','access_denied','cancelled','timeout']) {
    const fixture=light({connection:{port:1234,token}},{ok:false,error:{code,message:path+' '+token,effect_state:'unknown',method:'fs.stat'}});
    assert.throws(()=>fixture.ctx.fs.stat(path),fault(code));
  }
});

test('预备 Rust 宿主测试中的原始 JS 片段可执行，路径转义和边界断言一致', () => {
  const source=readFileSync(join(repo,'tools/js-engine-probe/tests/fs_stat_host.rs'),'utf8');
  const raw=source.match(/let script = r#"([\s\S]*?)"#;/)?.[1];
  assert(raw);
  const context={ctx:{},__hostCall:()=>{throw Error('词法测试不应发出宿主调用');}};
  const result=runInNewContext(desktop+browser+history+sdk+`; (function(ctx){${raw}})(ctx)`,context);
  assert.equal(result.rejected,result.total);
  assert.deepEqual(plain(result.codes),Array(3).fill('path_unavailable'));
  assert.deepEqual(plain(result.boundaries),Array(5).fill('fs_unavailable'));
  const format=source.match(/let script\s*=\s*format!\(\s*r#"([\s\S]*?)"#\s*,/)?.[1];
  assert(format, '须能从 rustfmt 格式化后的宿主测试读取原始脚本');
  const calls=[];
  const fs=createApi((_,params)=>{calls.push(params.path);return {kind:['file','directory','missing','other'][calls.length-1]};}).fs;
  const code=format.replace('{}',JSON.stringify(path)).replaceAll('{{','{').replaceAll('}}','}');
  const output=runInNewContext(`(function(ctx){${code}})(ctx)`,{ctx:{fs:{...fs,readText(){},writeText(){}}}});
  assert.deepEqual(plain(output.kinds),['file','directory','missing','other']);
  assert.deepEqual(calls,[path,'C:/','z:/folder/','C:\\中文😀']);
});

async function nodeInvocation(source,{handle,native=true,nativeError,cancel=false,timeout=10000,method='fs.stat'}={}) {
  const scratch=resolve(repo,'.tmp/parallel-fs/sdk');
  mkdirSync(scratch,{recursive:true});
  const directory=mkdtempSync(join(scratch,'node-'));
  const sockets=new Set();
  const errors=[];
  let child;
  const server=createServer(socket=>{
    sockets.add(socket);socket.once('close',()=>sockets.delete(socket));
    let input='',handled=false;
    socket.setEncoding('utf8');
    socket.on('data',chunk=>{
      input+=chunk;
      if(handled||!input.endsWith('\n'))return;
      handled=true;
      try {
        const frame=JSON.parse(input);
        assert.equal(frame.token,token);assert.equal(frame.method,method);
        handle?.(frame,socket);
        if(cancel)child.stdin.write('cancel\n');
      }catch(error){errors.push(error);socket.destroy();}
    });
  });
  let timer;
  try {
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
    const entry=join(directory,'entry.mjs'),response=join(directory,'response.json'),hostPath=join(directory,'host.mjs');
    writeFileSync(entry,source);writeFileSync(hostPath,desktop+browser+history+host);
    child=spawn(process.execPath,[hostPath],{stdio:['pipe','pipe','pipe'],windowsHide:true});
    let stderr='';child.stderr.on('data',chunk=>{stderr+=chunk;});
    const exited=once(child,'exit');timer=setTimeout(()=>child.kill(),timeout+3000);
    child.stdin.write(JSON.stringify({api_version:1,invocation:{invocation_id:'1',binding_id:'2'},input:{},entry_path:entry,
      package_root:directory,response_path:response,timeout_ms:timeout,
      ...(native?{native:{port:server.address().port,token}}:{}),...(nativeError?{native_error:nativeError}:{})})+'\n');
    const [code]=await exited;assert.equal(code,0,stderr);assert.deepEqual(errors,[]);
    const result=JSON.parse(readFileSync(response,'utf8'));
    assert(!JSON.stringify(result).includes(token));
    return result;
  }finally{
    clearTimeout(timer);if(child&&child.exitCode===null)child.kill();
    for(const socket of sockets)socket.destroy();
    await new Promise(resolve=>server.close(resolve));
    const target=realpathSync(directory),inside=relative(realpathSync(scratch),target);
    assert(inside&&!inside.startsWith('..')&&!isAbsolute(inside));rmSync(target,{recursive:true,force:true});
  }
}
const script=body=>`export default async ctx=>{${body}}`;
const call=`ctx.fs.stat(${JSON.stringify(path)})`;

test('Node 真实宿主的 Promise 回执、fs_busy 和精确路径，不借用桌面目标', {timeout:8000}, async()=>{
  let calls=0;
  const result=await nodeInvocation(script(`const first=${call};let busy;try{await ${call};}catch(e){busy={code:e.code,method:e.method,effect:e.effectState};}return {promise:typeof first.then==='function',value:await first,busy,hidden:ctx.__nativeContext===undefined};`),{handle:(frame,socket)=>{
    calls++;assert.deepEqual(frame.params,{path});socket.end(JSON.stringify({id:frame.id,ok:true,value:{kind:'file'}})+'\n');
  }});
  assert.equal(calls,1);assert.equal(result.status,'ok');
  assert.deepEqual(result.output,{promise:true,value:{kind:'file'},busy:{code:'fs_busy',method:'fs.stat',effect:'not_started'},hidden:true});
});

test('Node 无宿主与非法词法异步拒绝，不把访问错误转为 missing', {timeout:15000}, async()=>{
  for(const nativeError of [undefined,{code:'desktop_unavailable',message:path+' '+token,effect_state:'unknown'}]) {
    const result=await nodeInvocation(script(`return await ${call};`),{native:false,nativeError});
    assert.equal(result.error.code,'fs_unavailable');assert.equal(result.error.method,'fs.stat');assert.equal(result.error.effect_state,'not_started');
    assert(!result.error.message.includes('test-only-path-marker'));
  }
  const invalid=await nodeInvocation(script(`const value=ctx.fs.stat('C:/a/../b');try{await value;}catch(e){return {promise:typeof value.then==='function',code:e.code,method:e.method,effect:e.effectState};}`),{native:false});
  assert.deepEqual(invalid.output,{promise:true,code:'invalid_input',method:'fs.stat',effect:'not_started'});
  const denied=await nodeInvocation(script(`return await ${call};`),{handle:(frame,socket)=>socket.end(JSON.stringify({id:frame.id,ok:false,error:{code:'access_denied',message:path+' '+token,effect_state:'unknown'}})+'\n')});
  assert.equal(denied.status,'error');assert.equal(denied.error.code,'access_denied');assert.equal(denied.error.method,'fs.stat');assert.equal(denied.error.effect_state,'not_started');
  assert(!denied.error.message.includes('test-only-path-marker'));
});

test('Node 只读取消及外层时限保留 fs.stat 与 not_started', {timeout:8000},async()=>{
  for(const mode of ['cancel','timeout']){
    const result=await nodeInvocation(script(`return await ${call};`),{cancel:mode==='cancel',timeout:mode==='timeout'?150:3000});
    assert.equal(result.error.code,mode==='cancel'?'cancelled':'timeout');assert.equal(result.error.method,'fs.stat');assert.equal(result.error.effect_state,'not_started');
  }
});

test('Node 真实目录查询在请求发出后取消或到期，保留方法与只读效果', {timeout:8000},async()=>{
  for(const mode of ['cancel','timeout']) {
    let calls=0;
    const result=await nodeInvocation(script(`return await ctx.fs.readDirectory(${JSON.stringify(path)});`),{
      method:'fs.read_directory',cancel:mode==='cancel',timeout:mode==='timeout'?1000:3000,
      handle:frame=>{calls++;assert.deepEqual(frame.params,{path,max_entries:128});},
    });
    assert.equal(calls,1,'必须已进入该本机调用，不能以入口尚未运行的超时代替');
    assert.equal(result.error.code,mode==='cancel'?'cancelled':'timeout');
    assert.equal(result.error.method,'fs.read_directory');
    assert.equal(result.error.effect_state,'not_started');
  }
});

test('Node 零碎回执不能延长五秒总 RPC 时限，且不会重试', {timeout:10000},async()=>{
  let calls=0;
  const began=Date.now();
  const result=await nodeInvocation(script(`return await ${call};`),{handle:(_,socket)=>{
    calls++;const timer=setInterval(()=>socket.write(' '),300);socket.once('close',()=>clearInterval(timer));
  }});
  assert.equal(calls,1);assert.equal(result.error.code,'timeout');assert.equal(result.error.method,'fs.stat');assert.equal(result.error.effect_state,'not_started');
  assert(Date.now()-began<8000,'持续碎片不能把总时限延到动作时限');
});
