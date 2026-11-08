// 显式指定真实宿主；只使用自有回环TCP、模块目录及子进程，不操作桌面。
import assert from 'node:assert/strict';
import test from 'node:test';
import {spawn} from 'node:child_process';
import {createServer} from 'node:net';
import {mkdtempSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
const binary=process.env.OT_PROBE_BINARY;
assert(binary,'须设置 OT_PROBE_BINARY 为待验收宿主的绝对路径；不跳过测试。');
function run(t,request){
  const child=spawn(binary,[],{stdio:['pipe','pipe','pipe'],windowsHide:true});
  t.after(()=>{if(child.exitCode===null&&!child.killed)child.kill();});
  let stdout='',stderr='';child.stdout.setEncoding('utf8');child.stderr.setEncoding('utf8');
  child.stdout.on('data',data=>{stdout+=data;});child.stderr.on('data',data=>{stderr+=data;});
  child.stdin.on('error',()=>{});
  child.stdin.end(JSON.stringify({input:{},...request}));
  return new Promise((resolve,reject)=>{
    const timeout=setTimeout(()=>{child.kill();reject(Error('自有probe超过8秒测试预算'));},8000);
    child.once('error',error=>{clearTimeout(timeout);reject(error);});
    child.once('close',code=>{clearTimeout(timeout);try{resolve({code,...JSON.parse(stdout)});}catch(error){reject(Error(`回执不是JSON：${stderr}\n${stdout}`,{cause:error}));}});
  });
}
async function bridge(t){
  const sockets=new Set(),requests=[],waiters=[];const token='a'.repeat(64);
  const server=createServer(socket=>{
    sockets.add(socket);socket.on('close',()=>sockets.delete(socket));socket.on('error',()=>{});
    let bytes='';socket.on('data',chunk=>{
      bytes+=chunk; if(!bytes.includes('\n')||socket.requestRead)return;
      socket.requestRead=true;const request=JSON.parse(bytes);assert.equal(request.token,token);
      requests.push({...request,socket});for(const wake of waiters.splice(0))wake();
    });
  });
  await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',resolve);});
  t.after(async()=>{for(const socket of sockets)socket.destroy();await new Promise(resolve=>server.close(resolve));});
  const connection={port:server.address().port,token};
  const context={connection,browser_async_connection:connection,native_context_available:false};
  return {requests,sockets,context,
    wait:async count=>{
      const until=Date.now()+3000;
      while(requests.length<count){assert(Date.now()<until,'两个请求必须在任何完成回执前均已到达');await new Promise(resolve=>{const timer=setTimeout(resolve,20);waiters.push(()=>{clearTimeout(timer);resolve();});});}
    },
    complete:(request,value)=>request.socket.end(JSON.stringify({id:request.id,ok:true,value:{has_value:true,value}})+'\n'),
  };
}
const script=(body,native_context)=>({script:body,native_context,desktop_mode:'application_bridge'});
test('真实引擎返回Promise JSON和结构化拒绝',async t=>{
  const ok=await run(t,{script:'return Promise.resolve({value:7});'});assert.equal(ok.ok,true);assert.deepEqual(ok.result,{value:7});
  const bad=await run(t,{script:`return Promise.reject(Object.assign(new Error('denied'),{code:'permission_required',effectState:'not_started',method:'tabs.move',requiredPermissions:['tabs']}));`});
  assert.equal(bad.ok,false);assert.equal(bad.error.code,'permission_required');assert.equal(bad.error.effect_state,'not_started');assert.deepEqual(bad.error.required_permissions,['tabs']);
});
test('default async作者只通过具名main wrapper导入执行',async t=>{
  const folder=mkdtempSync(join(tmpdir(),'omnitrigger-script-async-'));
  writeFileSync(folder+'/author.mjs',`export default async function(ctx){return await Promise.resolve({wrapped:true});}`);
  writeFileSync(folder+'/wrapper.mjs',`import author from './author.mjs';export function main(ctx){return author(ctx);}`);
  const reply=await run(t,{package_root:folder,module_entry:'wrapper.mjs'});assert.equal(reply.ok,true);assert.deepEqual(reply.result,{wrapped:true});
  writeFileSync(folder+'/author-void.mjs',`export default async function(ctx){await Promise.resolve();}`);
  writeFileSync(folder+'/wrapper-void.mjs',`import author from './author-void.mjs';export function main(ctx){return author(ctx);}`);
  const voidReply=await run(t,{package_root:folder,module_entry:'wrapper-void.mjs'});assert.equal(voidReply.ok,true);assert.equal(voidReply.result,null);
});
test('真实独立TCP先派发两次，反序完成按ID回到Promise',async t=>{
  const host=await bridge(t);
  const result=run(t,script(`return(async()=>{const a=ctx.browser.callAsync('tabs.move',[1,{}]);const b=ctx.browser.callAsync('tabs.move',[2,{}]);return await Promise.all([a,b]);})();`,host.context));
  await host.wait(2);assert.notEqual(host.requests[0].id,host.requests[1].id);
  host.complete(host.requests.find(request=>request.params.args[0]===2),'second');host.complete(host.requests.find(request=>request.params.args[0]===1),'first');
  const reply=await result;assert.equal(reply.ok,true);assert.deepEqual(reply.result,['first','second']);
});
test('单独wrapper为default async作者提供异步browser视图，保留作者正文',async t=>{
  const host=await bridge(t);
  const folder=mkdtempSync(join(tmpdir(),'omnitrigger-script-async-'));
  writeFileSync(folder+'/author.mjs',`export default async function(ctx){return await Promise.all([ctx.browser.call('tabs.move',[1,{}]),ctx.browser.chrome.tabs.move(2,{})]);}`);
  writeFileSync(folder+'/wrapper.mjs',`import author from './author.mjs';export function main(ctx){const call=(...args)=>ctx.browser.callAsync(...args);const chrome=(parts=[])=>new Proxy(()=>{},{get:(_,key)=>key==='then'||key==='toJSON'||typeof key!=='string'?undefined:chrome([...parts,key]),apply:(_,self,args)=>call(parts.join('.'),args)});return author({...ctx,browser:Object.freeze({...ctx.browser,call,chrome:chrome()})});}`);
  const result=run(t,{module_entry:'wrapper.mjs',package_root:folder,native_context:host.context,desktop_mode:'application_bridge'});
  await host.wait(2);host.complete(host.requests.find(request=>request.params.args[0]===2),'second');host.complete(host.requests.find(request=>request.params.args[0]===1),'first');
  const reply=await result;assert.equal(reply.ok,true);assert.deepEqual(reply.result,['first','second']);
});
test('真实第五个在途not_started拒绝，不产生第五条TCP请求',async t=>{
  const host=await bridge(t);
  const result=run(t,script(`return(async()=>{const calls=[0,1,2,3].map(i=>ctx.browser.callAsync('tabs.move',[i,{}]));let fault;try{await ctx.browser.callAsync('tabs.move',[9,{}]);}catch(error){fault={code:error.code,effectState:error.effectState};}return{fault,values:await Promise.all(calls)};})();`,host.context));
  await host.wait(4);for(const request of host.requests)host.complete(request,request.params.args[0]);
  const reply=await result;assert.equal(reply.ok,true);assert.equal(host.requests.length,4);
  assert.deepEqual(reply.result.fault,{code:'browser_async_capacity',effectState:'not_started'});assert.deepEqual(reply.result.values,[0,1,2,3]);
});
test('旧父只有同步connection时异步拒绝且不联网',async t=>{
  const host=await bridge(t);const context={connection:host.context.connection,native_context_available:false};
  const reply=await run(t,script(`return ctx.browser.callAsync('tabs.move',[1,{}]);`,context));
  assert.equal(reply.ok,false);assert.equal(reply.error.code,'browser_async_unavailable');assert.equal(reply.error.effect_state,'not_started');assert.equal(host.requests.length,0);
});
test('主线程真实I/O等待超过2秒不扣JS CPU预算',async t=>{
  const host=await bridge(t);
  const result=run(t,script(`return ctx.browser.callAsync('tabs.move',[1,{}]);`,host.context));
  await host.wait(1);await new Promise(resolve=>setTimeout(resolve,2200));host.complete(host.requests[0],{waited:true});
  const reply=await result;assert.equal(reply.ok,true);assert.deepEqual(reply.result,{waited:true});
});
test('未await仍在途不能提前成功；自有连接及工作线程收尾',async t=>{
  const host=await bridge(t);
  const reply=await run(t,script(`ctx.browser.callAsync('tabs.move',[1,{}]);return{early:true};`,host.context));
  assert.equal(reply.ok,false);assert.equal(reply.error.code,'async_calls_pending');assert.equal(reply.error.effect_state,'unknown');
  const until=Date.now()+1000;while(host.sockets.size&&Date.now()<until)await new Promise(resolve=>setTimeout(resolve,10));assert.equal(host.sockets.size,0);
});
test('纯Pending Promise明确结束；自调度job链有预算',async t=>{
  const pending=await run(t,{script:'return new Promise(()=>{});'});assert.equal(pending.ok,false);assert.equal(pending.error.code,'script_promise_pending');
  const chain=await run(t,{script:'return new Promise(()=>{const again=()=>Promise.resolve().then(again);again();});'});assert.equal(chain.ok,false);assert.equal(chain.error.code,'script_budget_exceeded');
});
