import assert from 'node:assert/strict';
import test from 'node:test';
import {readFileSync} from 'node:fs';
import {prepareSelection} from './select-core.mjs';
import asyncMain from './select.mjs';
import {main as syncMain} from './select-script.mjs';

const input=(text,path=[],mode='inspect',format='compact')=>({text,path,mode,format});
const sample='{"items":[{"count":900719925474099312345,"s":"\\u0041","minus":-0,"exp":1.20e+03}],"__proto__":{"constructor":true}}';
async function run(asynchronous,value,receipt={submitted:true},failure) {
  const calls=[];
  const ctx={input:value,desktop:{sendText(text){calls.push(text);if(failure)throw failure;return asynchronous?Promise.resolve(receipt):receipt;}}};
  try{return {output:asynchronous?await asyncMain(ctx):syncMain(ctx),calls};}
  catch(error){return {error,calls};}
}

test('子值保留大整数、负零、指数和字符串token，不转换成JS数值',()=>{
  assert.equal(prepareSelection(input(sample,['items',0,'count'])).text,'900719925474099312345');
  assert.equal(prepareSelection(input(sample,['items',0,'minus'])).text,'-0');
  assert.equal(prepareSelection(input(sample,['items',0,'exp'])).text,'1.20e+03');
  assert.equal(prepareSelection(input(sample,['items',0,'s'])).text,'"\\u0041"');
});
test('对象键按解码后精确匹配，原型名和空键不触发属性访问',()=>{
  assert.equal(prepareSelection(input(sample,['__proto__','constructor'])).text,'true');
  assert.equal(prepareSelection(input('{"a\\u0062":{"":null}}',['ab',''])).text,'null');
  assert.equal(prepareSelection(input('{"a/b":{"~":[1]}}',['a/b','~',0])).text,'1');
  assert.equal({}.polluted,undefined);
});
test('空路径选择根、容器排版及子树节点数量准确',()=>{
  const root=prepareSelection(input(' [1,{"a":false}] ',[],'inspect','pretty'));
  assert.equal(root.text,'[\n  1,\n  {\n    "a": false\n  }\n]');
  assert.equal(root.summary.selectedKind,'array');assert.equal(root.summary.selectedNodeCount,4);
  assert.equal(prepareSelection(input('[[],{}]',[1])).text,'{}');
});
test('路径类型与缺项分开失败，不自动把数字键当数组索引',()=>{
  for(const [text,path,code] of [['{"0":1}',[0],'json_path_type'],['[1]',['0'],'json_path_type'],['[]',[0],'json_path_missing'],['{}',['x'],'json_path_missing'],['1',['x'],'json_path_type']])
    assert.throws(()=>prepareSelection(input(text,path)),error=>error.code===code);
});
test('未选分支仍完整校验，重复键和非法语法不能被路径绕过',()=>{
  for(const text of ['{"a":1,"b":{"x":0,"x":1}}','{"a":1,"b":[1,]}'])
    assert.throws(()=>prepareSelection(input(text,['a'])));
});
test('路径与输入有界，拒绝空洞、额外属性、未知字段和坏Unicode',()=>{
  const hole=Array(1);const extra=[];extra.x=1;
  for(const path of [hole,extra,Array(33).fill('a'),[-1],[1.5],[10000],['x'.repeat(1025)],['\ud800'],['a\0b']])
    assert.throws(()=>prepareSelection(input('{}',path)),error=>error.code==='invalid_input');
  assert.throws(()=>prepareSelection({...input('{}'),extra:true}),error=>error.code==='invalid_input');
  assert.throws(()=>prepareSelection(input('x'.repeat(65537))),error=>error.code==='invalid_input');
});
for(const asynchronous of [false,true]) {
  test(`${asynchronous?'Node':'script'} inspect只返回摘要，没有宿主调用或正文`,async()=>{
    const result=await run(asynchronous,input('{"private-key":"private-value"}',['private-key']));
    assert.equal(result.output.status,'inspected');assert.equal(result.output.selectedKind,'string');
    assert.deepEqual(result.calls,[]);assert(!JSON.stringify(result.output).includes('private'));
  });
  test(`${asynchronous?'Node':'script'} send只提交选中JSON一次，回执不含正文`,async()=>{
    const result=await run(asynchronous,input(sample,['items',0,'count'],'send'));
    assert.deepEqual(result.calls,['900719925474099312345']);
    assert.equal(result.output.inputSubmitted,true);assert(!JSON.stringify(result.output).includes('900719'));
  });
  test(`${asynchronous?'Node':'script'} 验证失败不调用宿主，输入失败保留状态不重试`,async()=>{
    const invalid=await run(asynchronous,input('{}',['secret'],'send'));
    assert.deepEqual(invalid.calls,[]);assert.equal(invalid.error.effectState,'not_started');
    assert(!invalid.error.message.includes('secret'));
    const failure=Object.assign(new Error('private-native-body'),{code:'target_changed',effectState:'not_started'});
    const failed=await run(asynchronous,input('[1]',[0],'send'),undefined,failure);
    assert.equal(failed.calls.length,1);assert.equal(failed.error.code,'target_changed');
    assert.equal(failed.error.effectState,'not_started');assert(!failed.error.message.includes('private'));
    const malformed=await run(asynchronous,input('1',[],'send'),{});
    assert.equal(malformed.calls.length,1);assert.equal(malformed.error.code,'invalid_response');assert.equal(malformed.error.effectState,'unknown');
  });
}
test('同步入口不会把Promise回执冒充完成',()=>{
  assert.throws(()=>syncMain({input:input('null',[],'send'),desktop:{sendText(){return Promise.resolve({submitted:true});}}}),error=>error.code==='invalid_response'&&error.effectState==='unknown');
});
test('新增动作和inspect示例保留旧动作身份与契约',()=>{
  const manifest=JSON.parse(readFileSync(new URL('action.json',import.meta.url),'utf8'));
  assert.equal(manifest.version,'1.1.0');assert.equal(manifest.actions.length,4);
  for(const name of ['select-call.json','select-script-call.json']) {
    const call=JSON.parse(readFileSync(new URL(name,import.meta.url),'utf8'));
    const action=manifest.actions.find(a=>a.id===call.action_id);
    assert.equal(call.package_id,manifest.id);assert.equal(action.version,call.action_version);
    assert.equal(action.single_instance,true);assert.equal(call.input.mode,'inspect');
  }
});
