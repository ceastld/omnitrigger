// 全部路径均为合成文本，不访问磁盘或真实输入宿主。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import {transform} from './transform.mjs';
import nodeMain from './main.mjs';
import {main as scriptMain} from './main-script.mjs';

const paths=['C:\\Example\\报告😀.tar.gz','D:/Data/.env','C:\\Example\\报告😀.tar.gz'];
async function run(script,input,{receipt={submitted:true},fault}={}){
  const calls=[];
  const invoke=text=>{calls.push(text);if(fault)throw fault;return receipt;};
  const ctx={input,desktop:{sendText:script?invoke:async text=>invoke(text)}};
  try{return{value:script?scriptMain(ctx):await nodeMain(ctx),calls};}catch(error){return{error,calls};}
}

test('名称与最后扩展名分解保留 Unicode、顺序、重复项和点开头名称',()=>{
  assert.equal(transform(paths,'name').text,'报告😀.tar.gz\n.env\n报告😀.tar.gz');
  assert.equal(transform(paths,'stem').text,'报告😀.tar\n.env\n报告😀.tar');
  assert.equal(transform(['C:\\a.b.c','C:\\plain'],'stem').text,'a.b\nplain');
});
test('父路径为明确词法规则，根与末尾分隔符不查询文件类型',()=>{
  assert.equal(transform(['C:\\','C:\\a','c:/a/b/'],'parent').text,'C:\\\nC:\\\nc:\\a');
  assert.equal(transform(['C:\\folder\\'],'name').text,'folder');
  assert.throws(()=>transform(['C:\\'],'name'),error=>error.code==='path_name_missing');
});
test('拒绝相对、UNC、设备、点段、ADS、重复分隔、禁用字符和无效 Unicode',()=>{
  for(const path of ['relative','C:relative','\\\\server\\share\\a','\\\\?\\C:\\a','C:\\a\\..\\b','C:\\.\\a','C:\\a\\\\b',
    'C:\\a:stream','C:\\a?','C:\\a.','C:\\a ','C:\\a\n','C:\\a\0','C:\\a\x85','C:\\a\ud800']){
    assert.throws(()=>transform([path],'name'),error=>error.code==='invalid_input');
  }
});
test('列表有界且不能静默忽略空洞或额外属性',()=>{
  for(const value of [[],Array(2),Object.assign(['C:\\a'],{extra:true}),Array(129).fill('C:\\a')]){
    assert.throws(()=>transform(value,'name'),error=>error.code==='invalid_input');
  }
  assert.equal(transform(['C:\\'+'a'.repeat(65533)],'name').summary.inputUtf16Units,65536);
  assert.equal(transform(['C:\\'+'a'.repeat(32765),'C:\\'+'b'.repeat(32765)],'name').summary.inputUtf16Units,65536);
  assert.throws(()=>transform(['C:\\'+'a'.repeat(32766),'C:\\'+'b'.repeat(32765)],'name'),error=>error.code==='input_too_large');
});

for(const script of [false,true]){
  const name=script?'轻量同步':'Node异步';
  test(`${name} inspect无宿主调用且终态不含路径或输出`,async()=>{
    const value=await run(script,{mode:'inspect',operation:'name',paths});
    assert.equal(value.value.status,'inspected');assert.equal(value.value.inputItems,3);assert.deepEqual(value.calls,[]);
    for(const secret of ['Example','报告','Data','.env'])assert.equal(JSON.stringify(value.value).includes(secret),false);
  });
  test(`${name} 发送一次完整结果，不读取其他宿主接口`,async()=>{
    const value=await run(script,{mode:'send',operation:'name',paths});
    assert.deepEqual(value.calls,[transform(paths,'name').text]);assert.equal(value.value.inputSubmitted,true);
    assert.equal(value.value.outputUtf16Units,value.calls[0].length);
  });
  test(`${name} 所有输入预验证，失败不发送`,async()=>{
    for(const input of [{mode:'send',operation:'name',paths:[...paths,'relative']},{mode:'send',operation:'unknown',paths},
      {mode:'send',operation:'name',paths,extra:1},{mode:'send',operation:'name',paths,[Symbol('hidden')]:1}]){
      const value=await run(script,input);assert.equal(value.error.code,'invalid_input');assert.deepEqual(value.calls,[]);
    }
  });
  test(`${name} 失败与缺失回执不重试，固定错误不包含路径`,async()=>{
    const input={mode:'send',operation:'name',paths};
    const fault=Object.assign(new Error(paths.join('私有')),{code:'target_changed',effectState:'not_started'});
    const value=await run(script,input,{fault});assert.equal(value.error.code,'target_changed');assert.equal(value.error.effectState,'not_started');
    assert.equal(value.calls.length,1);assert.equal(value.error.message.includes('私有'),false);
    const malformed=await run(script,input,{receipt:{}});
    assert.equal(malformed.error.code,'invalid_response');assert.equal(malformed.error.effectState,'unknown');assert.equal(malformed.calls.length,1);
  });
}
test('双入口声明与调用版本匹配，缺接口和异常 getter 不泄漏',async()=>{
  const manifest=JSON.parse(readFileSync(new URL('action.json',import.meta.url),'utf8'));
  for(const file of ['call.json','call-script.json','send-call.json']){
    const call=JSON.parse(readFileSync(new URL(file,import.meta.url),'utf8'));
    const action=manifest.actions.find(row=>row.id===call.action_id);
    assert.equal(call.package_id,manifest.id);assert.equal(call.action_version,action.version);assert.equal(action.single_instance,true);
  }
  const input={mode:'send',operation:'name',paths};
  assert.throws(()=>scriptMain({input}),error=>error.code==='desktop_unavailable'&&error.effectState==='not_started');
  assert.throws(()=>scriptMain({input,desktop:{}}),error=>error.code==='desktop_unavailable'&&error.effectState==='not_started');
  const fault={get code(){throw new Error('私有');}};
  const value=await run(true,input,{fault});assert.equal(value.error.code,'path_text_failed');assert.equal(value.error.message.includes('私有'),false);
});
