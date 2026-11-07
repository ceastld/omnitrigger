import assert from 'node:assert/strict';
import {test} from 'node:test';
import nodeMain from './main.mjs';
import {main as scriptMain} from './main-script.mjs';
const expected={message:'整点啦，起来喝杯水吧 💧',title:'喝水提醒',placement:'topRight',level:'info'};
const fail=(code,effectState)=>Object.assign(new Error('private'),{code,effectState,method:'wrong.method',requiredPermissions:['private']});
function make({input={},receipt={submitted:true},failure,onNotify}={}){
 const calls=[],controller=new AbortController();const ctx={input,signal:controller.signal,ui:{notify:options=>{calls.push(structuredClone(options));onNotify?.(options,controller);if(failure)throw failure;return receipt;}},clipboard:new Proxy({},{get(){throw Error('禁止访问剪贴板');}}),fs:new Proxy({},{get(){throw Error('禁止访问文件');}}),desktop:new Proxy({},{get(){throw Error('禁止输入或窗口操作');}})};return{ctx,calls,controller};
}
const check=(code,effect,method='ui.notify')=>error=>{assert.equal(error.code,code);assert.equal(error.effectState,effect);assert.equal(error.method,method);assert(!error.message.includes('private'));assert.equal(error.requiredPermissions,undefined);return true;};
for(const [runtime,main]of [['node',nodeMain],['script',scriptMain]]){
 const reject=async(env,predicate)=>runtime==='node'?assert.rejects(main(env.ctx),predicate):assert.throws(()=>main(env.ctx),predicate);
 test(`${runtime}：一次调用固定完整通知，无自建时长或调度`,async()=>{const env=make();assert.deepEqual(await main(env.ctx),{submitted:true});assert.deepEqual(env.calls,[expected]);assert.equal(Object.hasOwn(env.calls[0],'durationMs'),false);});
 test(`${runtime}：不接收隐藏参数，坏参数零通知`,async()=>{for(const input of [null,[],1,{message:'private'},{durationMs:60000}]){const env=make({input});await reject(env,check('invalid_input','not_started','script.parameters'));assert.equal(env.calls.length,0);}});
 test(`${runtime}：调用间不复用可被宿主改写的参数`,async()=>{const env=make({onNotify:o=>{o.message='private';}});await main(env.ctx);await main(env.ctx);assert.deepEqual(env.calls,[expected,expected]);});
 test(`${runtime}：无UI及宿主前置拒绝不报成功`,async()=>{const env=make();env.ctx.ui={};await reject(env,check('ui_unavailable','not_started'));assert.equal(env.calls.length,0);const denied=make({failure:fail('ui_busy','not_started')});await reject(denied,check('ui_busy','not_started'));assert.equal(denied.calls.length,1);});
 test(`${runtime}：未知效果/坏回执不重试，终态不含正文`,async()=>{for(const receipt of [null,{},false,{submitted:false},{submitted:true,message:'private'}]){const env=make({receipt});await reject(env,check('invalid_response','unknown'));assert.equal(env.calls.length,1);}const env=make({failure:fail('timeout','unknown')});await reject(env,check('timeout','unknown'));assert.equal(env.calls.length,1);});
}
test('Node：取消前零通知，通知返回后的取消始终unknown',async()=>{const pre=make();pre.controller.abort(fail('cancelled','not_started'));await assert.rejects(nodeMain(pre.ctx),check('cancelled','not_started'));assert.equal(pre.calls.length,0);for(const effect of ['not_started','unknown']){const env=make({onNotify:(_o,c)=>c.abort(fail('cancelled',effect))});await assert.rejects(nodeMain(env.ctx),check('cancelled','unknown'));assert.equal(env.calls.length,1);}});
