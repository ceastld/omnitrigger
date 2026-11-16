// 使用自有回环桥接和合成数据，不调用桌面、剪贴板或浏览器。
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once,EventEmitter} from 'node:events';
import {mkdirSync,mkdtempSync,readFileSync,realpathSync,rmSync,writeFileSync} from 'node:fs';
import {createServer} from 'node:net';
import {dirname,isAbsolute,join,relative,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {runInNewContext} from 'node:vm';
import test from 'node:test';

const repo=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const runtime=join(repo,'crates/omnitrigger-actions/runtime');
const host=readFileSync(join(runtime,'host.mjs'),'utf8');

test('真实Node宿主在调用时固定按键和嵌套时序，连接等待不改写请求', {timeout:20000}, async()=>{
  const scratch=join(repo,'.tmp/native-request-snapshot');
  mkdirSync(scratch,{recursive:true});
  const root=mkdtempSync(join(scratch,'owned-'));
  const sockets=new Set(),frames=[],errors=[];
  const server=createServer(socket=>{
    sockets.add(socket);socket.once('close',()=>sockets.delete(socket));
    let input='',handled=false;
    socket.setEncoding('utf8');
    socket.on('data',chunk=>{
      input+=chunk;if(handled||!input.endsWith('\n'))return;handled=true;
      try {
        const frame=JSON.parse(input);
        assert.equal(frame.token,'owned-synthetic-token');
        frames.push(frame);
        socket.end(JSON.stringify({id:frame.id,ok:true,value:{submitted:true}})+'\n');
      } catch(error) { errors.push(error);socket.destroy(); }
    });
  });
  let child,timer;
  try {
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
    const source=['desktop-api.js','browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js','state-api.js','host.mjs']
      .map(name=>readFileSync(join(runtime,name),'utf8')).join('\n');
    const entry=join(root,'entry.mjs'),hostPath=join(root,'host.mjs'),response=join(root,'response.json');
    writeFileSync(hostPath,source);
    writeFileSync(entry,`export default async ctx=>{
      const modifiers=[163],keys=[65];
      const first=ctx.desktop.sendKeySequence(modifiers,keys,{keyDownDelayMs:0});
      modifiers[0]=162;keys[0]=66;
      await first;
      const shortcut=[163,87],timing={key_down_delay_ms:10,process_overrides:{wps:20}};
      const second=ctx.desktop.sendShortcut(shortcut,{timing});
      shortcut[1]=81;timing.key_down_delay_ms=99;timing.process_overrides.wps=100;
      timing.process_overrides.other=1000;
      await second;
      return {completed:2};
    };`);
    child=spawn(process.execPath,[hostPath],{stdio:['pipe','pipe','pipe'],windowsHide:true});
    let stderr='';child.stderr.on('data',chunk=>{stderr+=chunk;});
    const exited=once(child,'exit');timer=setTimeout(()=>child.kill(),18000);
    child.stdin.write(JSON.stringify({api_version:1,invocation:{invocation_id:'1',binding_id:'2'},input:{},
      entry_path:entry,package_root:root,response_path:response,timeout_ms:15000,
      native:{port:server.address().port,token:'owned-synthetic-token'}})+'\n');
    const [code]=await exited;assert.equal(code,0,stderr);assert.deepEqual(errors,[]);
    assert.deepEqual(JSON.parse(readFileSync(response,'utf8')).output,{completed:2});
    assert.equal(frames.length,2);
    assert.deepEqual(frames.map(frame=>[frame.id,frame.method]),[[1,'keyboard.sequence'],[2,'keyboard.shortcut']]);
    assert.deepEqual(frames[0].params,{modifiers:[163],keys:[65],key_down_delay_ms:0});
    assert.deepEqual(frames[1].params,{keys:[163,87],timing:{key_down_delay_ms:10,process_overrides:{wps:20}}});
  } finally {
    clearTimeout(timer);if(child&&child.exitCode===null)child.kill();
    for(const socket of sockets)socket.destroy();
    await new Promise(resolve=>server.close(resolve));
    const target=realpathSync(root),inside=relative(realpathSync(scratch),target);
    assert(inside&&!inside.startsWith('..')&&!isAbsolute(inside));
    rmSync(target,{recursive:true,force:true});
  }
});

test('不可编码参数在连接前结构化拒绝，不泄露正文且不占用后续请求', async()=>{
  const body=host.slice(host.indexOf('function nativeCall('),host.indexOf('const desktopApis ='));
  for(const kind of ['circular','getter']) {
    const sockets=[];
    class Socket extends EventEmitter {
      setEncoding() {} setTimeout() {} destroy() {}
      write(frame) {this.frame=JSON.parse(frame);}
    }
    const controller=new AbortController();
    const scope={Buffer,TextDecoder,Promise,Set,controller,nativeMethods:new Set(['keyboard.sequence']),
      nativeSockets:new Set(),nativeId:0,nativePending:undefined,nativePendingMethod:undefined,
      request:{native:{port:1,token:'synthetic'}},
      fault:(code,message)=>Object.assign(Error(message),{code}),checkCancelled:()=>{},
      createConnection:()=>{const socket=new Socket();sockets.push(socket);return socket;},
      setTimeout:()=>1,clearTimeout:()=>{},
    };
    const call=runInNewContext(body+'\nnativeCall;',scope);
    const params={};
    if(kind==='circular')params.self=params;
    else Object.defineProperty(params,'text',{enumerable:true,get(){throw Error('PRIVATE_SYNTHETIC_BODY');}});
    const result=call('keyboard.sequence',params);result.catch(()=>{});
    assert.equal(sockets.length,0);
    await assert.rejects(result,error=>error.code==='invalid_input'&&error.method==='keyboard.sequence'
      &&error.effectState==='not_started'&&!error.message.includes('PRIVATE'));
    const next=call('keyboard.sequence',{modifiers:[],keys:[65],key_down_delay_ms:0});
    assert.equal(sockets.length,1);sockets[0].emit('connect');
    sockets[0].emit('data',JSON.stringify({id:sockets[0].frame.id,ok:true,value:{submitted:true}})+'\n');
    assert.equal((await next).submitted,true);
  }
});
