import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
const source=readFileSync(new URL('../crates/omnitrigger-actions/runtime/browser-api.js',import.meta.url),'utf8');
const create=Function(source+'\nreturn createBrowserApi;')();

test('浏览器调用在等待连接前固定嵌套JSON和选项，不冻结调用方',async()=>{
  let release;
  const connected=new Promise(resolve=>{release=resolve;});
  const api=create(async(method,args,options)=>{
    await connected;
    return {method,args,options};
  },fn=>async(...args)=>fn(...args));
  const nested={windowId:7,active:false,metadata:{names:['original']}};
  const args=[nested],options={profile:'owned',timeoutMs:1000};
  const pending=api.call('tabs.create',args,options);
  nested.windowId=8;nested.active=true;nested.metadata.names[0]='changed';args.push(123);
  options.profile='other';options.timeoutMs=60000;release();
  assert.deepEqual(await pending,{method:'tabs.create',args:[{windowId:7,active:false,metadata:{names:['original']}}],options:{profile:'owned',timeoutMs:1000}});
  assert.equal(Object.isFrozen(nested),false);assert.equal(Object.isFrozen(options),false);
});

test('同步适配器也只持有参数副本，JSON特殊键保留为普通数据',()=>{
  let captured;
  const api=create((method,args,options)=>{captured={method,args,options};return true;});
  const data=JSON.parse('{"__proto__":{"value":"data"},"constructor":"literal","items":[null,"中😀"]}');
  const args=[data],options={profile:'owned'};
  assert.equal(api.call('tabs.create',args,options),true);
  captured.args[0].items.push('adapter-change');captured.args[0].__proto__.value='changed';
  captured.options.profile='adapter-profile';
  assert.deepEqual(data.items,[null,'中😀']);assert.equal(data.__proto__.value,'data');
  assert.equal(options.profile,'owned');
  assert(Object.hasOwn(captured.args[0],'__proto__'));
  assert.equal(Object.getPrototypeOf(captured.args[0]),Object.prototype);
  assert.equal(captured.args[0].constructor,'literal');
});

test('profiles同样在调用时固定选项，非法JSON仍在invoke前拒绝',async()=>{
  let calls=0,selected;
  const api=create((_method,_args,options)=>{calls++;selected=options;return [];});
  const options={profile:'owned',timeoutMs:1234};
  assert.deepEqual(api.profiles(options),[]);options.profile='changed';
  assert.deepEqual(selected,{profile:'owned',timeoutMs:1234});
  const circular={};circular.self=circular;
  for(const args of [[circular],[undefined],[{nested:[()=>{}]}]]) {
    assert.throws(()=>api.call('tabs.create',args),error=>error.code==='invalid_request'&&error.effectState==='not_started');
  }
  assert.equal(calls,1);
});
