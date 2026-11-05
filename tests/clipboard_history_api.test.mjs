// 仅用内存合成历史和共享工厂；不启动宿主、监听器，不读取系统或个人数据。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';

const source = readFileSync(new URL('../crates/omnitrigger-actions/runtime/clipboard-history-api.js', import.meta.url), 'utf8');
const createApi = runInNewContext(source + '\ncreateClipboardHistoryApi');
const plain = value => JSON.parse(JSON.stringify(value));
const stamp = '2026-01-01T00:00:00Z';
const secret = 'synthetic-history-private-payload-😀';
const maxU64 = '18446744073709551615';
const metadata = {id:maxU64, preview:'合成😀', utf16Units:4, createdAt:stamp};
const replies = {
  list:{items:[metadata], revision:maxU64},
  get:{id:maxU64, text:secret, createdAt:stamp, revision:maxU64},
  append:{id:maxU64, revision:maxU64, inserted:true},
  remove:{removed:true, revision:maxU64},
};
const inputs = {list:[{limit:200}], get:[{id:maxU64}], append:[{text:secret}], remove:[{id:maxU64}]};
const fault = (method, code, effect) => error => {
  assert.equal(error.method, 'history.' + method);
  assert.equal(error.code, code);
  assert.equal(error.effectState, effect);
  assert(!String(error.message).includes(secret));
  assert.equal(error.requiredPermissions, undefined);
  assert.equal(error.text, undefined);
  return true;
};
function factory(reply, asynchronous = false) {
  const calls = [];
  const api = createApi((method, params) => {
    calls.push([method, plain(params)]);
    if (reply instanceof Error) {
      if (asynchronous) return Promise.reject(reply);
      throw reply;
    }
    return asynchronous ? Promise.resolve(reply) : reply;
  });
  return {api, calls};
}
const write = method => ['append','remove'].includes(method);

test('四方法同步和 Promise 结果严格映射，不发生额外桥接或自动重试', async () => {
  for (const asynchronous of [false,true]) for (const method of Object.keys(replies)) {
    const {api,calls} = factory(replies[method], asynchronous);
    const value = api[method](...inputs[method]);
    assert.equal(typeof value?.then === 'function', asynchronous);
    assert.deepEqual(plain(await value), replies[method]);
    assert.deepEqual(calls, [['history.' + method, inputs[method][0]]]);
    assert.equal(Object.isFrozen(api), true);
  }
});

test('列表默认 100，边界 1/200；摘要独立复制且不携带全文', () => {
  for (const args of [[],[undefined],[{}],[{limit:1}],[{limit:200}]]) {
    const {api,calls} = factory(replies.list);
    const value = api.list(...args);
    assert.equal(calls[0][1].limit, args[0]?.limit ?? 100);
    assert.equal(Object.hasOwn(value.items[0], 'text'), false);
    assert.notEqual(value.items, replies.list.items);
    assert.notEqual(value.items[0], metadata);
    value.items[0].preview = '修改副本';
    assert.equal(metadata.preview, '合成😀');
  }
  assert.deepEqual(plain(factory({items:[],revision:'0'}).api.list()), {items:[],revision:'0'});
});

test('显式保存空文本、空白、控制字符及 UTF-16 上限；参数不 trim', () => {
  for (const text of ['', ' \r\n\t ', '\u0001', 'x'.repeat(65536), '😀'.repeat(32768)]) {
    const {api,calls} = factory({...replies.append,inserted:false});
    assert.equal(api.append({text}).inserted, false);
    assert.equal(calls[0][1].text, text);
    assert.equal(calls.length, 1);
  }
});

test('输入拒绝精度损失、未知字段和 getter 异常，非法输入不调用宿主', () => {
  const invalidIds = ['', '0','00','01','-1','+1','1.0',' 1','1 ', '1e3','18446744073709551616',maxU64+'0',1,1n,null];
  const bad = {
    list:[[null],[[]],[1],[{limit:0}],[{limit:201}],[{limit:1.5}],[{limit:NaN}],[{limit:'1'}],[{extra:secret}],[{},{}]],
    get:[[],[maxU64],...[...invalidIds].map(id => [{id}]),[{id:'1',extra:secret}],[{id:'1'},{}]],
    append:[[],[secret],[null],[{text:'\0'}],[{text:'\ud800'}],[{text:'\udfff'}],[{text:'x'.repeat(65537)}],[{text:'😀'.repeat(32769)}],[{text:1}],[{text:''},{}]],
    remove:[[],...[...invalidIds].map(id => [{id}]),[{id:'1',text:secret}]],
  };
  let calls = 0;
  const api = createApi(() => {calls++; throw Error('must-not-call');});
  for (const [method, groups] of Object.entries(bad)) {
    for (const args of groups) assert.throws(() => api[method](...args), fault(method,'invalid_input','not_started'));
    const key = method === 'list' ? 'limit' : method === 'append' ? 'text' : 'id';
    const poisoned = Object.defineProperty({},key,{get(){throw Error(secret);}});
    assert.throws(() => api[method](poisoned),fault(method,'invalid_input','not_started'));
    for (const extra of [Symbol('extra'),'extra']) {
      const data = method === 'list' ? {} : {...inputs[method][0]};
      Object.defineProperty(data,extra,{value:secret});
      assert.throws(() => api[method](data),fault(method,'invalid_input','not_started'));
    }
  }
  assert.equal(calls,0);
});

test('结果拒绝未知字段、缺字段、u64 溢出和畸形 revision，不回显正文', async () => {
  for (const method of Object.keys(replies)) {
    const bad = [null,[],{}, {...replies[method],extra:secret}];
    for (const revision of ['', '00','01','-1','18446744073709551616',1,null]) bad.push({...replies[method],revision});
    const hidden = Object.defineProperty({...replies[method]},'extra',{value:secret});
    bad.push(hidden);
    const getter = Object.defineProperty({...replies[method]},'revision',{get(){throw Error(secret);}});
    bad.push(getter);
    for (const key of Object.keys(replies[method])) {
      const missing = {...replies[method]}; delete missing[key]; bad.push(missing);
    }
    for (const value of bad) {
      const effect = write(method) ? 'unknown' : 'not_started';
      assert.throws(() => factory(value).api[method](...inputs[method]),fault(method,'invalid_response',effect));
      await assert.rejects(factory(value,true).api[method](...inputs[method]),fault(method,'invalid_response',effect));
    }
  }
});

test('列表摘要严格单行与代理对边界，条目不重复、不稀疏、不超过请求数量', () => {
  const bad = [
    {...metadata,text:secret}, {...metadata,id:'0'}, {...metadata,id:'01'},
    ...[-1,65537,1.1,'4',NaN].map(utf16Units => ({...metadata,utf16Units})),
    ...['x'.repeat(65),'😀'.repeat(33),'\ud800','\udfff','\0','a\nb','a\rb','\u2028','\u2029','\u0085','\v','\f'].map(preview => ({...metadata,preview})),
    {...metadata,createdAt:1}, {...metadata,createdAt:'\ud800'},
  ];
  for (const item of bad) assert.throws(() => factory({items:[item],revision:'1'}).api.list(), fault('list','invalid_response','not_started'));
  for (const items of [[metadata,metadata],Array(1),Object.assign([metadata],{extra:secret}),Array(201).fill(metadata)]) {
    assert.throws(() => factory({items,revision:'1'}).api.list(),fault('list','invalid_response','not_started'));
  }
  const two = [metadata,{...metadata,id:'1'}];
  assert.throws(() => factory({items:two,revision:'1'}).api.list({limit:1}),fault('list','invalid_response','not_started'));
  for (const preview of ['', 'x'.repeat(64),'😀'.repeat(32),' a\tb ']) {
    assert.equal(factory({items:[{...metadata,preview,utf16Units:64}],revision:'1'}).api.list().items[0].preview,preview);
  }
  assert.throws(() => factory({items:[{...metadata,utf16Units:0}],revision:'1'}).api.list(),fault('list','invalid_response','not_started'));
  const capacity = Array.from({length:17},(_,index) => ({...metadata,id:String(index+1),utf16Units:65536}));
  assert.equal(factory({items:capacity.slice(0,16),revision:'17'}).api.list().items.length,16);
  assert.throws(() => factory({items:capacity,revision:'17'}).api.list(),fault('list','invalid_response','not_started'));
});

test('get 校验请求身份及全文边界；append/remove 必须返回 boolean', () => {
  for (const value of [{...replies.get,id:'1'}, ...['\0','\ud800','x'.repeat(65537)].map(text => ({...replies.get,text}))]) {
    assert.throws(() => factory(value).api.get(...inputs.get),fault('get','invalid_response','not_started'));
  }
  for (const method of ['append','remove']) {
    const key = method === 'append' ? 'inserted' : 'removed';
    for (const value of [0,1,'false',null,undefined]) {
      assert.throws(() => factory({...replies[method],[key]:value}).api[method](...inputs[method]),fault(method,'invalid_response','unknown'));
    }
    assert.equal(factory({...replies[method],[key]:false}).api[method](...inputs[method])[key],false);
  }
});

test('桥接异常脱敏，读操作无副作用；写入不确定时保守 unknown，不重试', async () => {
  for (const method of Object.keys(replies)) for (const effect of ['not_started','unknown',undefined,'bad']) {
    const error = Object.assign(new Error(secret),{code:'history_item_missing',effectState:effect,method:secret,requiredPermissions:[secret],text:secret});
    const expected = write(method) && effect !== 'not_started' ? 'unknown' : 'not_started';
    const {api,calls} = factory(error);
    assert.throws(() => api[method](...inputs[method]),fault(method,'history_item_missing',expected));
    assert.equal(calls.length,1);
    await assert.rejects(factory(error,true).api[method](...inputs[method]),fault(method,'history_item_missing',expected));
  }
  for (const code of ['native_bridge_unavailable','desktop_unavailable']) {
    assert.throws(() => factory(Object.assign(Error(secret),{code,effectState:'not_started'})).api.append({text:''}),fault('append','history_unavailable','not_started'));
  }
  const poisoned = Object.defineProperty(Error(secret),'code',{get(){throw Error(secret);}});
  assert.throws(() => factory(poisoned).api.get(...inputs.get),fault('get','history_failed','not_started'));
  for (const code of ['synthetic_private_history_text',secret,'history_unrecognized_payload']) {
    const error = Object.assign(Error(secret),{code,text:secret});
    assert.throws(() => factory(error).api.append({text:''}),fault('append','history_failed','unknown'));
  }
});

test('无桥接时四方法仍存在并明确 unavailable；不调用系统剪贴板或监听器', () => {
  const api = createApi();
  for (const method of Object.keys(replies)) assert.throws(() => api[method](...inputs[method]),fault(method,'history_unavailable','not_started'));
});

test('合成存储序列保留空白、连续去重、稳定字符串 ID 与无变化 revision', () => {
  let revision = 0n, nextId = 9007199254740993n;
  const rows = [];
  const calls = [];
  const api = createApi((method, params) => {
    calls.push(method);
    if (method === 'history.append') {
      const last = rows.at(-1);
      if (last && last.text === params.text) return {id:last.id,revision:String(revision),inserted:false};
      const item = {id:String(nextId++),text:params.text,createdAt:stamp}; rows.push(item); revision++;
      return {id:item.id,revision:String(revision),inserted:true};
    }
    if (method === 'history.list') return {items:rows.slice(-params.limit).reverse().map(({id,text,createdAt}) => ({id,preview:text,utf16Units:text.length,createdAt})),revision:String(revision)};
    if (method === 'history.get') {
      const item = rows.find(item => item.id === params.id);
      if (!item) throw Object.assign(Error(secret),{code:'history_item_missing'});
      return {...item,revision:String(revision)};
    }
    const index = rows.findIndex(item => item.id === params.id);
    if (index < 0) return {removed:false,revision:String(revision)};
    rows.splice(index,1); revision++;
    return {removed:true,revision:String(revision)};
  });
  const first = api.append({text:'  合成😀  '});
  assert.equal(first.id,'9007199254740993');
  assert.equal(api.append({text:'  合成😀  '}).inserted,false);
  assert.equal(api.get({id:first.id}).text,'  合成😀  ');
  assert.equal(api.list().items.length,1);
  assert.equal(api.remove({id:'1'}).revision,'1');
  assert.equal(api.remove({id:first.id}).revision,'2');
  assert.throws(() => api.get({id:first.id}),fault('get','history_item_missing','not_started'));
  assert.equal(api.list().items.length,0);
  assert.equal(calls.length,8);
});

test('200 条摘要及 Unicode 边界生成数据保持精度，读取全文只返回请求条目', () => {
  const items = Array.from({length:200},(_,index) => ({id:String(9007199254740993n+BigInt(index)),
    preview:'😀'.repeat(32),utf16Units:4096,createdAt:'0'}));
  const value = factory({items,revision:maxU64}).api.list({limit:200});
  assert.equal(value.items.length,200);
  assert.equal(value.items[199].id,'9007199254741192');
  for (let index=0;index<=64;index++) {
    const text = 'a'.repeat(index)+'😀'+'合成\r\n';
    const result = factory({...replies.get,text}).api.get(...inputs.get);
    assert.equal(result.text,text);
    assert.equal(result.id,maxU64);
  }
  for (const preview of ['a'.repeat(62)+'😀','a'.repeat(63)]) {
    const item={...metadata,preview,utf16Units:64};
    assert.equal(factory({items:[item],revision:'1'}).api.list().items[0].preview,preview);
  }
  assert.throws(() => factory({items:[{...metadata,preview:'a'.repeat(63)+'😀'}],revision:'1'}).api.list(),fault('list','invalid_response','not_started'));
});
