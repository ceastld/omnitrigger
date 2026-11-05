// 生产通知入口与共享SDK回归；桥接仅记录合成参数，不显示UI或创建调度。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
import node from '../examples/actions/script-notification/main.mjs';
const root=new URL('../',import.meta.url);
const script=runInNewContext(readFileSync(new URL('examples/actions/script-notification/main.js',root),'utf8')+'\nmain;');
const createDesktopApi=Function(readFileSync(new URL('crates/omnitrigger-actions/runtime/desktop-api.js',root),'utf8')+'\nreturn createDesktopApi;')();
async function run(main,asynchronous,input,{fault}={}){
  const calls=[];const api=createDesktopApi((method,params)=>{calls.push({method,params});if(fault)throw fault;return{submitted:true};});
  const ctx={input,ui:{notify:asynchronous?async params=>api.ui.notify(params):api.ui.notify}};
  for(const key of ['desktop','clipboard','process','fs','browser','delay'])Object.defineProperty(ctx,key,{get(){throw new Error('禁止访问 '+key);}});
  try{return{value:await main(ctx),calls};}catch(error){return{error,calls};}
}
const input={message:'合成提醒😀'};
for(const [name,main,asynchronous] of [['轻量同步',script,false],['Node异步',node,true]]){
  test(`${name} 完整输入交给SDK，默认值和合法旧参数行为不变`,async()=>{
    const result=await run(main,asynchronous,input);
    assert.deepEqual(result.value,{submitted:true});
    assert.deepEqual(result.calls,[{method:'ui.notify',params:{message:input.message,title:'OmniTrigger',level:'info',placement:'topRight',durationMs:3000}}]);
    const explicit={...input,title:'合成标题',placement:'center',level:'warning',durationMs:1000};
    const value=await run(main,asynchronous,Object.freeze(explicit));assert.deepEqual(value.calls[0].params,explicit);
  });
  test(`${name} 未知字段/非枚举字段/Symbol在通知前拒绝，不读取额外getter`,async()=>{
    for(const value of [{...input,extra:true},Object.defineProperty({...input},'extra',{get(){throw new Error('不应读取');}}),
      {...input,[Symbol('extra')]:true}]){
      const result=await run(main,asynchronous,value);assert.equal(result.error.code,'invalid_input');
      assert.equal(result.error.method,'ui.notify');assert.equal(result.error.effectState,'not_started');assert.deepEqual(result.calls,[]);
    }
  });
  test(`${name} 正文和时长边界保留SDK规则，参数坏时无桥接`,async()=>{
    assert.equal((await run(main,asynchronous,{message:'😀'.repeat(512)})).calls.length,1);
    for(const value of [{message:'😀'.repeat(513)},{message:' \n '},{message:'x\0y'},{message:'\ud800'},
      {...input,durationMs:999},{...input,durationMs:30001},null,[]]){
      const result=await run(main,asynchronous,value);assert.equal(result.error.code,'invalid_input');assert.deepEqual(result.calls,[]);
    }
  });
  test(`${name} UI失败/效果未知原样保留且不重试，不等待显示或创建定时器`,async()=>{
    const fault=Object.assign(new Error('合成UI失败'),{code:'ui_error',method:'ui.notify',effectState:'unknown'});
    const result=await run(main,asynchronous,input,{fault});assert.equal(result.error,fault);assert.equal(result.calls.length,1);
  });
}
test('两种生产入口对相同合法参数和参数错误保持同一契约',async()=>{
  for(const input of [{message:'合成通知'}, {message:'合成通知',title:'合成标题',level:'success',placement:'bottomRight',durationMs:1200}]){
    assert.deepEqual(await run(script,false,input),await run(node,true,input));
  }
  for(const input of [{message:'合成通知',unknown:true},{message:'x\0y'},null]){
    const sync=await run(script,false,input);const asynchronous=await run(node,true,input);
    assert.deepEqual(sync.calls,[]);assert.deepEqual(asynchronous.calls,[]);
    for(const value of [sync,asynchronous]){
      assert.equal(value.error.code,'invalid_input');assert.equal(value.error.method,'ui.notify');assert.equal(value.error.effectState,'not_started');
    }
  }
});
