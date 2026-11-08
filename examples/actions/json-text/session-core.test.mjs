import test from 'node:test';
import assert from 'node:assert/strict';
import {createSession,transition,view,encodeEditValue,pointer} from './session-core.mjs';

const create=(text,continuous=true)=>createSession({sessionId:'demo',text,continuous});
const step=(s,type,extra={})=>transition(s,{type,revision:s.revision,...extra});
const select=(s,path)=>step(s,'select',{path}).state;
const begin=(s,path)=>step(select(s,path),'begin_edit').state;
const bad=(fn,code)=>assert.throws(fn,e=>e.code===code&&e.effectState!=='unknown');
const ack=(s,status)=>transition(s,{type:'ack_output',requestId:s.pending.id,status});

test('真实编辑/导航/输出参数序列：Int64不经过JS数值，路径不依赖点号拼接',()=>{
  const original=create('{"n":9007199254740993,"rows":[{"a/b~c":true},null]}');
  const saved=JSON.stringify(original);let s=begin(original,['n']);
  const replacement=encodeEditValue({kind:'int64',value:'9223372036854775807'});
  s=step(s,'commit_edit',{replacementText:replacement}).state;
  assert.equal(s.text,'{"n":9223372036854775807,"rows":[{"a/b~c":true},null]}');
  assert.equal(s.revision,1);assert.equal(JSON.stringify(original),saved);
  const value=step(s,'request_output',{kind:'value',format:'compact'});
  assert.equal(value.effects[0].text,'9223372036854775807');
  s=ack(value.state,'confirmed').state;assert.equal(s.mode,'browse');
  s=step(select(s,['rows']),'open_selected').state;
  s=step(select(s,['rows',0]),'open_selected').state;
  assert.deepEqual(view(s).rows.map(x=>x.path),[['rows',0,'a/b~c']]);
  const output=step(select(s,['rows',0,'a/b~c']),'request_output',{kind:'path',format:'pointer'});
  assert.equal(output.effects[0].text,'/rows/0/a~1b~0c');
  s=ack(output.state,'unknown').state;assert.equal(s.mode,'closed');assert.equal(s.endReason,'output_unknown');
  bad(()=>step(s,'request_output',{kind:'value',format:'compact'}),'session_closed');
});

for(const [text,key,expected] of [
  ['{"a":1,"b":2,"c":3}','a','{"b":2,"c":3}'],
  ['{"a":1,"b":2,"c":3}','b','{"a":1,"c":3}'],
  ['{"a":1,"b":2,"c":3}','c','{"a":1,"b":2}'],
  ['{"a":1}','a','{}'],
  ['[9007199254740993,2,3]',0,'[2,3]'],
  ['[9007199254740993,2,3]',1,'[9007199254740993,3]'],
  ['[9007199254740993,2,3]',2,'[9007199254740993,2]'],
  ['[null]',0,'[]'],
])test(`删除第${String(key)}项不会损坏相邻JSON或大整数`,()=>{
  const before=create(text),s=step(select(before,[key]),'delete_selected').state;
  assert.equal(s.text,expected);assert.equal(s.selectionPath,null);assert.equal(s.revision,1);assert.equal(before.text,text);
  bad(()=>transition(s,{type:'select',revision:0,path:[key]}),'stale_revision');
});

test('用户键仅作为数据：点号/括号/原型名称不会成为表达式或对象属性赋值',()=>{
  let s=create('{"__proto__":{"polluted":false},"a.b[0]":{"constructor":7}}');
  s=step(select(s,['__proto__']),'open_selected').state;
  s=begin(s,['__proto__','polluted']);s=step(s,'commit_edit',{replacementText:'true'}).state;
  assert.equal(s.text,'{"__proto__":{"polluted":true},"a.b[0]":{"constructor":7}}');
  assert.equal({}.polluted,undefined);s=step(s,'home').state;
  s=step(select(s,['a.b[0]']),'open_selected').state;
  assert.equal(view(s).pointer,'/a.b[0]');assert.equal(view(s).rows[0].label,'constructor');
});

test('表单取消保持文档；错误提交或旧revision不产生任何修改',()=>{
  const before=begin(create('{"a":1}'),['a']),raw=JSON.stringify(before);
  bad(()=>step(before,'commit_edit',{replacementText:'not json'}),'invalid_json');
  bad(()=>transition(before,{type:'commit_edit',revision:99,replacementText:'2'}),'stale_revision');
  bad(()=>step(before,'home'),'edit_pending');assert.equal(JSON.stringify(before),raw);
  const cancelled=step(before,'cancel_edit').state;assert.equal(cancelled.text,'{"a":1}');assert.equal(cancelled.revision,0);assert.equal(cancelled.mode,'browse');
});

test('编辑空对象、数组、假值、null与原始字符串具有明确JSON类型',()=>{
  let s=create('{"a":0}');
  for(const [kind,value,expected] of [['object','{}','{}'],['array','[]','[]'],['boolean',false,'false'],['null',null,'null'],['string','  😀\n','"  😀\\n"']]) {
    s=begin(s,['a']);s=step(s,'commit_edit',{replacementText:encodeEditValue({kind,value})}).state;
    assert.equal(s.text,'{"a":'+expected+'}');
  }
});

for(const value of ['9223372036854775807','-9223372036854775808','9007199254740993','0','-0'])test(`Int64字面保持精确：${value}`,()=>{
  assert.equal(encodeEditValue({kind:'int64',value}),value);
});
for(const value of ['9223372036854775808','-9223372036854775809','1.2',' 1','1e2'])test(`Int64不隐式截断或转换：${value}`,()=>{
  bad(()=>encodeEditValue({kind:'int64',value}),'invalid_int64');
});
test('任意JSON数值保留词法，不经double或Date文化转换',()=>{
  assert.equal(encodeEditValue({kind:'number',value:'1e100000'}),'1e100000');
  bad(()=>encodeEditValue({kind:'Date',value:'01/02/2026'}),'unsupported_edit_kind');
  bad(()=>encodeEditValue({kind:'boolean',value:'True'}),'invalid_edit_value');
  bad(()=>encodeEditValue({kind:'object',value:'[]'}),'edit_kind_mismatch');
});

test('复制请求未获确认前不关闭、不发第二次请求',()=>{
  const chosen=select(create('{"a":"hello"}',false),['a']);
  const out=step(chosen,'request_output',{kind:'value',format:'raw-string'});
  assert.deepEqual(out.effects,[{type:'text_output_requested',requestId:'demo:1',revision:0,text:'hello',payloadFormat:'raw-string'}]);
  assert.equal(out.state.mode,'pending');
  bad(()=>step(out.state,'request_output',{kind:'value',format:'raw-string'}),'output_pending');
  const done=ack(out.state,'confirmed');assert.equal(done.state.mode,'closed');assert.equal(done.effects.length,0);
  bad(()=>ack(out.state,'Succeeded'),'invalid_output_ack');
  bad(()=>transition(out.state,{type:'ack_output',requestId:'other:1',status:'confirmed'}),'output_ack_mismatch');
});

test('明确not_started可返回浏览；unknown关闭并保留未知效果，不自动重试',()=>{
  const chosen=select(create('{"a":null}'),['a']);const pending=step(chosen,'request_output',{kind:'value',format:'compact'}).state;
  const notStarted=ack(pending,'not_started');assert.equal(notStarted.state.mode,'browse');assert.equal(notStarted.effects.length,0);
  const unknown=ack(pending,'unknown');assert.equal(unknown.state.mode,'closed');assert.equal(unknown.effects.length,0);
  assert.equal(unknown.state.lastOutput.status,'unknown');
});

test('取消后晚到确认只记录结果，不能恢复或再次发出输出',()=>{
  const pending=step(select(create('{"x":1}'),['x']),'request_output',{kind:'value',format:'compact'}).state;
  const cancelled=transition(pending,{type:'cancel'});assert.equal(cancelled.state.mode,'closed');assert.notEqual(cancelled.state.pending,null);
  const late=ack(cancelled.state,'confirmed');assert.equal(late.state.mode,'closed');assert.equal(late.state.endReason,'cancelled');
  assert.equal(late.state.lastOutput.status,'confirmed');assert.equal(late.effects.length,0);
  assert.equal(transition(late.state,{type:'cancel'}).state.endReason,'cancelled');
});

test('新JSON明确替换并重置导航；导出整文档在确认后退出',()=>{
  let s=step(select(create('{"a":{}}'),['a']),'open_selected').state;
  s=step(s,'load_document',{text:'[9007199254740993,false]'}).state;
  assert.equal(s.revision,1);assert.deepEqual(s.browsePath,[]);assert.equal(s.selectionPath,null);
  const out=step(s,'request_output',{kind:'document',format:'compact'});
  assert.equal(out.effects[0].text,'[9007199254740993,false]');assert.equal(out.state.mode,'pending');
  assert.equal(ack(out.state,'confirmed').state.mode,'closed');
});
test('整文档导出保持JSON语法，不能以raw-string去除根字符串引号',()=>{
  const s=create('"hello"');bad(()=>step(s,'request_output',{kind:'document',format:'raw-string'}),'unsupported_output_format');
  assert.equal(step(s,'request_output',{kind:'document',format:'compact'}).effects[0].text,'"hello"');
});

for(const text of ['false','null','""','9007199254740993'])test(`根标量有明确只读行与输出：${text}`,()=>{
  const s=create(text),v=view(s);assert.equal(v.rows.length,1);assert.deepEqual(v.rows[0].path,[]);
  const chosen=select(s,[]);bad(()=>step(chosen,'delete_selected'),'cannot_delete_root');
  assert.equal(step(chosen,'request_output',{kind:'value',format:'compact'}).effects[0].text,text);
});
test('空对象/数组可浏览，取消不会生成输出',()=>{
  for(const text of ['{}','[]']) {
    const s=create(text);assert.equal(view(s).rows.length,0);
    assert.equal(transition(s,{type:'cancel'}).effects.length,0);
  }
});
test('不把缺值、坏JSON或重复键静默变为空对象；不添加来源未完成的add操作',()=>{
  for(const text of ['', '{', '[1,]','{"x":1,"x":2}'])assert.throws(()=>create(text));
  const s=create('{"a":1}');bad(()=>step(s,'add',{value:'2'}),'unsupported_operation');
  bad(()=>step(s,'select',{path:['not-here']}),'path_missing');
  bad(()=>step(s,'select',{path:['a','nested']}),'path_type_mismatch');
});
test('输出使用明确路径格式，不把Pointer冒充原JSON-Path；无法传输的原始文本明确拒绝',()=>{
  const s=select(create('{"a":"\\u0000","b":"\\ud800"}'),['a']);
  bad(()=>step(s,'request_output',{kind:'path',format:'jsonpath'}),'unsupported_output_format');
  bad(()=>step(s,'request_output',{kind:'value',format:'raw-string'}),'output_not_transportable');
  const b=select(create('{"b":"\\ud800"}'),['b']);bad(()=>step(b,'request_output',{kind:'value',format:'raw-string'}),'output_not_transportable');
  assert.equal(step(s,'request_output',{kind:'value',format:'compact'}).effects[0].text,'"\\u0000"');
  assert.equal(pointer(['a/b','~','dot.key',0]),'/a~1b/~0/dot.key/0');
});
