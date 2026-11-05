// 仅合成文本与 RecordingHost，不读取选区或剪贴板。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import {transform} from './transform.mjs';
import nodeMain from './main.mjs';
import {main as scriptMain} from './main-script.mjs';

const source='私有乙\r\n私有甲\n私有乙\r私有甲\r\n';
async function run(script,input,{receipt={submitted:true},fault,onCall}={}){
  const calls=[];
  const invoke=text=>{calls.push(text);onCall?.();if(fault)throw fault;return receipt;};
  const ctx={input,desktop:{sendText:script?invoke:async text=>invoke(text)}};
  try{return{value:script?scriptMain(ctx):await nodeMain(ctx),calls};}catch(error){return{error,calls};}
}

test('保序去重不 trim、不折叠大小写或 Unicode 形式',()=>{
  const value=transform('b\na\nb\nA\n a\né\ne\u0301\na\n','deduplicate');
  assert.equal(value.text,'b\na\nA\n a\né\ne\u0301\n');
  assert.equal(value.summary.removedLines,2);assert.equal(value.summary.inputLines,8);
  assert.equal(transform(value.text,'deduplicate').text,value.text);
});
test('排序固定按 UTF16码元，不作自然数字排序且保留重复',()=>{
  assert.equal(transform('a\n2\n10\nA\na\n\ue000\n😀','sort').text,'10\n2\nA\na\na\n😀\n\ue000');
  const value=transform(source,'sort');
  assert.equal(value.summary.removedLines,0);assert.equal(transform(value.text,'sort').text,value.text);
});
test('混合换行统一LF，并保留终止换行的明确语义',()=>{
  for(const [text,expected,inputLines] of [['','',0],['\n','\n',1],['\r\n','\n',1],['a\r\nb\ra','a\nb',3],
    ['a\n\na\n','a\n\n',3],['\n\n','\n',2],['a\r\n','a\n',1]]){
    const result=transform(text,'deduplicate');assert.equal(result.text,expected);assert.equal(result.summary.inputLines,inputLines);
    assert.equal(result.summary.terminalNewline,/\r|\n/.test(text.slice(-1)));
  }
});
test('UTF16与行数边界明确，无效代理项和NUL拒绝',()=>{
  assert.equal(transform('😀'.repeat(32768),'sort').summary.outputUtf16Units,65536);
  assert.throws(()=>transform('x'.repeat(65537),'sort'),error=>error.code==='input_too_large');
  assert.equal(transform('x\n'.repeat(10000),'sort').summary.inputLines,10000);
  assert.throws(()=>transform('x\n'.repeat(10001),'sort'),error=>error.code==='too_many_lines');
  for(const text of ['x\0y','\ud800','\udfff','x\ud800a'])assert.throws(()=>transform(text,'deduplicate'),error=>error.code==='invalid_input');
});

for(const script of [false,true]){
  const name=script?'轻量同步':'Node异步';
  test(`${name} inspect不访问宿主，摘要不回显正文`,async()=>{
    const value=await run(script,{mode:'inspect',operation:'deduplicate',text:source});
    assert.equal(value.value.status,'inspected');assert.equal(value.value.removedLines,2);assert.deepEqual(value.calls,[]);
    assert.equal(JSON.stringify(value.value).includes('私有'),false);
  });
  test(`${name} 单次完整发送，空文本无提交而换行仍提交`,async()=>{
    const input={mode:'send',operation:'deduplicate',text:source};
    const value=await run(script,input,{onCall:()=>{input.text='后续变更';input.operation='sort';}});
    assert.deepEqual(value.calls,['私有乙\n私有甲\n']);assert.equal(value.value.outputLines,2);assert.equal(value.value.inputSubmitted,true);
    const empty=await run(script,{mode:'send',operation:'sort',text:''});
    assert.equal(empty.value.status,'empty');assert.equal(empty.value.inputSubmitted,false);assert.deepEqual(empty.calls,[]);
    const newline=await run(script,{mode:'send',operation:'sort',text:'\n'});assert.deepEqual(newline.calls,['\n']);
  });
  test(`${name} 非法参数先拒绝，取消与回执异常不重试`,async()=>{
    for(const input of [{mode:'send',operation:'unknown',text:source},{mode:'send',operation:'sort',text:source,extra:true},
      {mode:'send',operation:'sort',text:source,[Symbol('extra')]:true},{mode:'send',operation:'sort',text:'\ud800'}]){
      const value=await run(script,input);assert.equal(value.error.code,'invalid_input');assert.deepEqual(value.calls,[]);
    }
    const input={mode:'send',operation:'sort',text:source};
    const fault=Object.assign(new Error(source),{code:'cancelled',effectState:'unknown'});
    const cancelled=await run(script,input,{fault});assert.equal(cancelled.error.code,'cancelled');assert.equal(cancelled.error.effectState,'unknown');
    assert.equal(cancelled.error.message.includes('私有'),false);assert.equal(cancelled.calls.length,1);
    const malformed=await run(script,input,{receipt:{submitted:true,extra:'私有'}});
    assert.equal(malformed.error.code,'invalid_response');assert.equal(malformed.error.effectState,'unknown');assert.equal(malformed.calls.length,1);
  });
}
test('缺接口与恶意错误getter输出固定消息和可判断效果',async()=>{
  const input={mode:'send',operation:'sort',text:source};
  assert.throws(()=>scriptMain({input}),error=>error.code==='desktop_unavailable'&&error.effectState==='not_started');
  const fault={get code(){throw new Error(source);}};
  const value=await run(true,input,{fault});assert.equal(value.error.code,'line_text_failed');assert.equal(value.error.effectState,'unknown');
  assert.equal(value.error.message.includes('私有'),false);
});
test('清单与调用版本匹配，inspect默认且双入口结果相同',async()=>{
  const manifest=JSON.parse(readFileSync(new URL('action.json',import.meta.url),'utf8'));
  for(const file of ['call.json','call-script.json','send-call.json']){
    const call=JSON.parse(readFileSync(new URL(file,import.meta.url),'utf8'));
    const action=manifest.actions.find(row=>row.id===call.action_id);
    assert.equal(call.package_id,manifest.id);assert.equal(call.action_version,action.version);assert.equal(action.single_instance,true);
    assert.equal(call.input.mode,file==='send-call.json'?'send':'inspect');
    assert.deepEqual(await run(true,call.input),await run(false,call.input));
  }
});
