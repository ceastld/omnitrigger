import test from 'node:test';
import assert from 'node:assert/strict';
import {main} from './session-step-script.mjs';

const initial=(text='{"n":9007199254740993,"a":[false,null]}')=>({sessionId:'api_demo',text,continuous:true});
const call=input=>main(new Proxy({input},{get(target,key){assert.equal(key,'input','入口只能读取input，禁止访问Native/UI/状态存储');return target.input;}}));
const create=(text,page)=>call({mode:'create',initial:initial(text),...(page?{page}:{} )});
const round=value=>JSON.parse(JSON.stringify(value));
const event=(s,type,extra={})=>({type,revision:s.revision,...extra});
const step=(s,type,extra={})=>call({mode:'step',state:round(s),event:event(s,type,extra)});
const bad=(fn,code)=>assert.throws(fn,e=>e.code===code&&e.method==='script.parameters'&&e.effectState==='not_started'&&!e.message.includes('SECRET'));

test('普通作者入口支持逐轮可序列化state/event，不触碰其它ctx能力',()=>{
  const created=create();assert.equal(created.status,'created');assert.equal(created.effectsExecuted,false);
  let s=round(created.state),before=JSON.stringify(s);
  s=step(s,'select',{path:['n']}).state;
  s=step(s,'begin_edit').state;
  s=step(s,'commit_edit',{replacementText:'9223372036854775807'}).state;
  assert.equal(s.text,'{"n":9223372036854775807,"a":[false,null]}');assert.equal(s.revision,1);
  assert.equal(JSON.parse(before).text,'{"n":9007199254740993,"a":[false,null]}');
  const output=step(s,'request_output',{kind:'document',format:'compact'});
  assert.equal(output.requests[0].text,s.text);assert.equal(output.effectsExecuted,false);
  const done=call({mode:'step',state:round(output.state),event:{type:'ack_output',requestId:output.state.pending.id,status:'confirmed'}});
  assert.equal(done.state.mode,'closed');assert.equal(done.requests.length,0);assert.equal(done.effectsExecuted,false);
});

test('旧revision、缺事件与额外入口参数明确拒绝；不返回改写状态',()=>{
  const s=step(create().state,'select',{path:['n']}).state;const before=JSON.stringify(s);
  bad(()=>call({mode:'step',state:s,event:{type:'select',revision:99,path:['a']}}),'stale_revision');
  bad(()=>call({mode:'step',state:s}),'invalid_input');
  bad(()=>call({mode:'create',initial:initial(),execute:true}),'invalid_input');
  bad(()=>step(s,'not-real'),'unsupported_operation');assert.equal(JSON.stringify(s),before);
});

test('坏JSON与重复键保持结构化错误，不把SECRET输入带进错误消息',()=>{
  bad(()=>create('{"SECRET":'), 'invalid_json');
  bad(()=>create('{"SECRET":1,"SECRET":2}'),'json_duplicate_key');
  bad(()=>call({mode:'create',initial:{sessionId:'bad',text:{n:9007199254740993},continuous:true}}),'invalid_input');
});

test('分页只影响view，文档保持完整；原四入口不需要改变',()=>{
  const doc=JSON.stringify(Object.fromEntries(Array.from({length:70},(_,i)=>['k'+i,i])));
  const a=create(doc);assert.equal(a.view.totalRows,70);assert.equal(a.view.rows.length,32);assert.equal(a.state.text,doc);
  const b=create(doc,{offset:64,limit:6});assert.equal(b.view.rows[0].label,'k64');assert.equal(b.view.rows.length,6);assert.equal(b.state.text,doc);
  const c=create(doc,{offset:70,limit:1});assert.deepEqual(c.view.rows,[]);
  assert.equal(b.view.previewsAreSummaries,true);assert.equal(b.view.previewLimit,160);
});
for(const page of [{offset:0,limit:0},{offset:0,limit:65},{offset:-1,limit:1},{offset:1.2,limit:1},{offset:10001,limit:1},{offset:0,limit:1,x:1}])test(`无效分页拒绝 ${JSON.stringify(page)}`,()=>{
  bad(()=>create('{}',page),'invalid_page');
});
test('超过现有行数的offset不静默回退',()=>bad(()=>create('{}',{offset:1,limit:1}),'invalid_page'));

test('根字符串预览有界，文档与请求正文不截断',()=>{
  const text=JSON.stringify('😀'.repeat(200));const out=create(text);
  assert.equal(Array.from(out.view.rows[0].preview).length,160);assert.equal(out.state.text,text);
  const s=step(out.state,'select',{path:[]}).state;
  const request=step(s,'request_output',{kind:'value',format:'raw-string'});
  assert.equal(request.requests[0].text,'😀'.repeat(200));
});

test('UTF8结果预算在纯作者层拒绝，不能截断state或已提出的正文',()=>{
  const doc=JSON.stringify({text:'界'.repeat(12000)}); // 约36KiB，初始state可返回。
  const s=step(create(doc).state,'select',{path:['text']}).state;const before=JSON.stringify(s);
  bad(()=>step(s,'request_output',{kind:'value',format:'raw-string'}),'session_output_too_large');
  assert.equal(JSON.stringify(s),before);assert.equal(s.pending,null);
});
test('输入预算不扩大ScriptHost请求上限',()=>bad(()=>create(JSON.stringify('x'.repeat(110000))),'session_input_too_large'));

test('结果不能带回无法再次提交的pending状态，失败保持原state',()=>{
  const doc=JSON.stringify({payload:'a'.repeat(12000),padding:'b'.repeat(38000)});
  const s=step(create(doc).state,'select',{path:['payload']}).state,before=JSON.stringify(s);
  bad(()=>step(s,'request_output',{kind:'value',format:'raw-string'}),'session_state_too_large');
  assert.equal(JSON.stringify(s),before);assert.equal(s.pending,null);
});

test('unknown和取消后晚到确认保持纯模型语义，不生成额外输出',()=>{
  const s=step(create('{"value":"own"}').state,'select',{path:['value']}).state;
  const p=step(s,'request_output',{kind:'value',format:'raw-string'}).state;
  const cancelled=call({mode:'step',state:round(p),event:{type:'cancel'}});
  const late=call({mode:'step',state:round(cancelled.state),event:{type:'ack_output',requestId:p.pending.id,status:'confirmed'}});
  assert.equal(late.state.mode,'closed');assert.equal(late.state.endReason,'cancelled');assert.equal(late.requests.length,0);assert.equal(late.effectsExecuted,false);
});

for(const [label,mutate] of [
  ['BigInt',()=>({mode:'create',initial:{...initial(),continuous:1n}})],
  ['NaN',()=>({mode:'create',initial:{...initial(),continuous:NaN}})],
  ['function',()=>({mode:'create',initial:{...initial(),continuous:()=>true}})],
  ['cycle',()=>{const value={mode:'create'};value.initial=value;return value;}],
  ['getter',()=>Object.defineProperty({mode:'create'},'initial',{get(){throw Error('SECRET getter必须不运行');},enumerable:true})],
  ['sparse-array-extra',()=>{const a=Array(1);a.extra='x';return {mode:'step',state:a,event:{}};}],
])test(`只接收序列化JSON数据：${label}`,()=>bad(()=>call(mutate()),'non_serializable_input'));

test('JSON共享对象引用可序列化，不因引用相同伪报循环',()=>{
  const shared={};bad(()=>call({mode:'step',state:shared,event:shared}),'invalid_state');
});
