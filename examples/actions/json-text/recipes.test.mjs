import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import {analyze,format} from './json.mjs';
import asyncMain from './main.mjs';
import {main as syncMain} from './main-script.mjs';

const source = '{ "count": 900719925474099312345678901234567890, "negative": -0, "exponent": 1.2300e+05, "name": "文😀\\u0041" }';
async function run(asynchronous,input,{choice={status:'selected',id:'pretty'},receipt={submitted:true},failAt,error}={}) {
  const calls=[];
  const call=(method,args,result)=>{
    calls.push([method,args]);
    const perform=()=>{if(method===failAt)throw error;return result;};
    return asynchronous?Promise.resolve().then(perform):perform();
  };
  const ctx={input,ui:{choose:options=>call('ui.choose',[options],choice)},
    desktop:{sendText:text=>call('keyboard.text',[text],receipt)}};
  try{return{output:asynchronous?await asyncMain(ctx):syncMain(ctx),calls};}
  catch(error){return{error,calls};}
}

test('排版保留大整数、负零、指数和字符串原 token',()=>{
  const parsed=analyze(source);
  const compact=format(parsed.tokens,'compact');
  assert.equal(compact,'{"count":900719925474099312345678901234567890,"negative":-0,"exponent":1.2300e+05,"name":"文😀\\u0041"}');
  const pretty=format(parsed.tokens,'pretty');
  assert.ok(pretty.includes('900719925474099312345678901234567890'));
  assert.ok(pretty.includes('1.2300e+05'));
  assert.ok(pretty.includes('"文😀\\u0041"'));
  assert.equal(parsed.summary.numberCount,3);
});

test('合法语法与排版往返，包括标量、空容器和转义 NUL 文本',()=>{
  const values=['null','true','false','""','"a\\u0000b"','[]','{}','[{},[],true,false,null,-1.2e-3]',
    '{"__proto__":{"polluted":true},"escaped\\\"key":"\\/"}'];
  for(const raw of values){
    const parsed=analyze(raw);
    for(const style of ['pretty','compact']){
      const output=format(parsed.tokens,style);
      assert.deepEqual(JSON.parse(output),JSON.parse(raw));
      assert.equal(output.includes('\0'),false);
    }
  }
  assert.equal({}.polluted,undefined);
});

test('确定性合成结构的两种排版保持相同 JSON 值',()=>{
  for(let index=0;index<100;index++){
    const value={index,array:[index%2===0,null,{['key'+index]:'文😀\n\\"'+index}],value:index/10};
    const raw=JSON.stringify(value,null,index%4);
    const parsed=analyze(raw);
    assert.deepEqual(JSON.parse(format(parsed.tokens,'compact')),value);
    assert.deepEqual(JSON.parse(format(parsed.tokens,'pretty')),value);
  }
});

test('坏语法、重复解码键和资源边界均明确拒绝',()=>{
  for(const raw of ['', ' ', '\ufeff{}','[1,]','{"a":1,}','{a:1}','[','{"a" 1}',
    '00','-01','1.','1e','1e+','--1','Infinity','NaN','true false','//comment\n{}','"raw\nline"','"\\x20"']){
    assert.throws(()=>analyze(raw),error=>error.code==='invalid_json');
  }
  assert.throws(()=>analyze('{"a":1,"\\u0061":2}'),error=>error.code==='json_duplicate_key');
  assert.equal(analyze('['.repeat(32)+'0'+']'.repeat(32)).summary.maxDepth,32);
  assert.throws(()=>analyze('['.repeat(33)+'0'+']'.repeat(33)),error=>error.code==='json_limit');
  assert.equal(analyze('['+Array(9999).fill('0').join(',')+']').summary.nodeCount,10000);
  assert.throws(()=>analyze('['+Array(10000).fill('0').join(',')+']'),error=>error.code==='json_limit');
  assert.equal(analyze('"'+'a'.repeat(65534)+'"').summary.inputUtf16Units,65536);
  for(const raw of ['"\ud800"','"\udfff"','\0','"'+'a'.repeat(65535)+'"']){
    assert.throws(()=>analyze(raw),error=>error.code==='invalid_input');
  }
});

for(const asynchronous of [false,true]){
  const name=asynchronous?'Node':'同步';
  test(`${name} inspect 无宿主访问，终态仅摘要`,async()=>{
    const result=await run(asynchronous,{mode:'inspect',text:source});
    assert.deepEqual(result.calls,[]);
    assert.equal(result.output.status,'inspected');
    assert.equal(result.output.inputSubmitted,false);
    assert.equal(JSON.stringify(result.output).includes('9007199254740993'),false);
    assert.equal(result.output.rootKind,'object');
  });

  test(`${name} 显式发送和选择发送都只提交一次原目标文本`,async()=>{
    for(const style of ['compact','pretty']){
      const result=await run(asynchronous,{mode:'send',format:style,text:source});
      assert.equal(result.output.status,'submitted');
      assert.equal(result.output.format,style);
      assert.deepEqual(result.calls,[['keyboard.text',[format(analyze(source).tokens,style)]]]);
    }
    const selected=await run(asynchronous,{mode:'choose-send',text:source},{choice:{status:'selected',id:'compact'}});
    assert.deepEqual(selected.calls.map(call=>call[0]),['ui.choose','keyboard.text']);
    assert.equal(selected.output.format,'compact');
    const cancelled=await run(asynchronous,{mode:'choose-send',text:source},{choice:{status:'cancelled',reason:'dismissed'}});
    assert.equal(cancelled.output.status,'cancelled');
    assert.equal(cancelled.calls.length,1);
  });

  test(`${name} 参数、语法和排版大小错误先于选择和输入`,async()=>{
    for(const input of [{mode:'read'}, {mode:'inspect',text:'{}',extra:1}, {mode:'send',text:'{}'},
      {mode:'send',format:'unknown',text:'{}'}, {mode:'choose-send',text:'{bad'},
      {mode:'inspect',text:'{"secret-private-key":1,"secret-private-key":2}'}]){
      const result=await run(asynchronous,input);
      assert.ok(result.error);
      assert.equal(result.error.message.includes('secret-private-key'),false);
      assert.deepEqual(result.calls,[]);
    }
    const large='['+Array(9999).fill('1000').join(',')+']';
    const inspect=await run(asynchronous,{mode:'inspect',text:large});
    assert.equal(inspect.output.status,'inspected');
    const rejected=await run(asynchronous,{mode:'choose-send',text:large});
    assert.equal(rejected.error.code,'json_output_too_large');
    assert.deepEqual(rejected.calls,[]);
    const compact=await run(asynchronous,{mode:'send',format:'compact',text:large});
    assert.equal(compact.output.inputSubmitted,true);
  });

  test(`${name} 未知选择、失焦、取消与坏回执均无重试或内容泄漏`,async()=>{
    const bad=await run(asynchronous,{mode:'choose-send',text:source},{choice:{status:'selected',id:'external-command'}});
    assert.equal(bad.error.code,'invalid_response');
    assert.equal(bad.calls.length,1);
    for(const code of ['target_changed','cancelled','timeout']){
      const error=Object.assign(new Error(source),{code,effectState:'not_started'});
      const result=await run(asynchronous,{mode:'send',format:'pretty',text:source},{failAt:'keyboard.text',error});
      assert.equal(result.error.code,code);
      assert.equal(result.error.message.includes(source),false);
      assert.equal(result.calls.length,1);
    }
    const result=await run(asynchronous,{mode:'send',format:'compact',text:'{}'},{receipt:{}});
    assert.equal(result.error.effectState,'unknown');
    assert.equal(result.calls.length,1);
  });
}

test('动作清单和四个调用示例引用同一共享模块契约',()=>{
  const manifest=JSON.parse(readFileSync(new URL('action.json',import.meta.url),'utf8'));
  assert.deepEqual(manifest.actions.filter(action=>['inspect-or-send','inspect-or-send-script'].includes(action.id)).map(action=>action.runtime),['node','script']);
  for(const name of ['call.json','call-script.json','send-call.json','choose-call.json']){
    const call=JSON.parse(readFileSync(new URL(name,import.meta.url),'utf8'));
    const action=manifest.actions.find(action=>action.id===call.action_id);
    assert.equal(call.package_id,manifest.id);
    assert.equal(action.version,call.action_version);
    assert.equal(action.single_instance,true);
  }
});
