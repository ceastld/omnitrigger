import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
import asyncMain from './main.mjs';
import {main as syncMain} from './main-script.mjs';
import {prepare} from './workflow.mjs';

const secret=' 合成文本 文😀\n\t ';
const receipt={id:'9007199254740993',revision:'18446744073709551615',inserted:true};
async function run(asynchronous,input,{data,append=receipt,failAt,error}={}){
  const calls=[];
  const copy=input?.mode==='copy';
  data??={kind:'text',text:secret,sequenceNumber:0xffffffff,...(copy?{changed:true}:{})};
  const invoke=(method,args,value)=>{
    calls.push([method,args]);
    const perform=()=>{if(method===failAt)throw error;return value;};
    return asynchronous?Promise.resolve().then(perform):perform();
  };
  const ctx={input,clipboard:{readData:(...args)=>invoke('clipboard.read_data',args,data),
    history:{append:(...args)=>invoke('history.append',args,append)}},
    desktop:{copySelectionData:(...args)=>invoke('selection.copy_data',args,data)}};
  try{return{output:asynchronous?await asyncMain(ctx):syncMain(ctx),calls};}
  catch(error){return{error,calls};}
}

for(const asynchronous of [false,true]){
  const name=asynchronous?'Node':'同步';
  test(`${name} 显式read和原目标copy后只写一次文本历史`,async()=>{
    for(const mode of ['read','copy']){
      const result=await run(asynchronous,{mode});
      assert.deepEqual(result.calls.map(row=>row[0]),[mode==='read'?'clipboard.read_data':'selection.copy_data','history.append']);
      assert.deepEqual(result.calls[1][1],[{text:secret}]);
      assert.deepEqual(result.output,{operation:mode==='read'?'read-to-history':'copy-to-history',saved:true,
        inserted:true,utf16Units:secret.length,...(mode==='copy'?{changed:true}:{})});
      const serialized=JSON.stringify(result.output);
      for(const privateValue of [secret,'9007199254740993','18446744073709551615'])assert.equal(serialized.includes(privateValue),false);
    }
  });

  test(`${name} 空文本、空白和连续重复由历史API处理，不trim`,async()=>{
    for(const text of ['', ' \r\n\t ', '😀'.repeat(32768)]){
      const result=await run(asynchronous,{mode:'read'},
        {data:{kind:'text',text,sequenceNumber:0},append:{...receipt,inserted:false}});
      assert.deepEqual(result.calls[1][1],[{text}]);
      assert.equal(result.output.inserted,false);
      assert.equal(result.output.utf16Units,text.length);
    }
  });

  test(`${name} 参数与等待选项错误先于全部宿主操作`,async()=>{
    const inputs=[null,[],{}, {mode:'append'}, {mode:'read',options:{}}, {mode:'copy',extra:1},
      ...[null,[],{extra:true},{timeoutMs:0},{timeoutMs:5001},{timeoutMs:1},{pollIntervalMs:0},
        {pollIntervalMs:1001},{settleMs:-1},{settleMs:1001},{allowUnchanged:'true'},
        {timeoutMs:undefined}].map(options=>({mode:'copy',options}))];
    for(const input of inputs){
      const result=await run(asynchronous,input);
      assert.equal(result.error?.code,'invalid_input');
      assert.deepEqual(result.calls,[]);
    }
  });

  test(`${name} 只有显式允许未变化数据才保存，400/10不隐式重试复制`,async()=>{
    const data={kind:'text',text:secret,sequenceNumber:7,changed:false};
    const rejected=await run(asynchronous,{mode:'copy'},{data});
    assert.equal(rejected.error.code,'selection_unchanged');
    assert.equal(rejected.error.effectState,'unknown');
    assert.equal(rejected.calls.length,1);
    const options={timeoutMs:400,pollIntervalMs:10,settleMs:0,allowUnchanged:true};
    const accepted=await run(asynchronous,{mode:'copy',options},{data});
    assert.equal(accepted.output.changed,false);
    assert.deepEqual(accepted.calls[0][1],[options]);
    assert.equal(accepted.calls.length,2);
  });

  test(`${name} 文件列表和坏格式绝不转字符串或保存`,async()=>{
    for(const mode of ['read','copy']){
      for(const paths of [[],['C:\\synthetic-secret.txt']]){
        const result=await run(asynchronous,{mode},{data:{kind:'files',paths,sequenceNumber:0,...(mode==='copy'?{changed:true}:{})}});
        assert.equal(result.error.code,'history_text_required');
        assert.equal(result.error.effectState,mode==='copy'?'unknown':'not_started');
        assert.equal(result.error.message.includes('synthetic-secret'),false);
        assert.equal(result.calls.length,1);
      }
    }
    for(const data of [{kind:'image',sequenceNumber:0},{kind:'text',text:'\ud800',sequenceNumber:0},
      {kind:'text',text:'a\0b',sequenceNumber:0},{kind:'text',text:'ok',sequenceNumber:-1},
      {kind:'text',text:'ok',sequenceNumber:4294967296},{kind:'text',text:'ok',sequenceNumber:0,extra:1}]){
      const result=await run(asynchronous,{mode:'read'},{data});
      assert.equal(result.error.code,'invalid_response');
      assert.equal(result.calls.length,1);
    }
  });

  test(`${name} 取消和写入失败保留先前复制效果，不回退或重试`,async()=>{
    const fault=Object.assign(new Error(secret),{code:'cancelled',effectState:'not_started'});
    for(const mode of ['read','copy']){
      const result=await run(asynchronous,{mode},{failAt:'history.append',error:fault});
      assert.equal(result.error.code,'cancelled');
      assert.equal(result.error.effectState,mode==='copy'?'unknown':'not_started');
      assert.equal(result.error.message.includes(secret),false);
      assert.equal(result.calls.length,2);
    }
    const before=await run(asynchronous,{mode:'copy'},{failAt:'selection.copy_data',error:fault});
    assert.equal(before.error.effectState,'not_started');
    assert.equal(before.calls.length,1);
    const malformed=await run(asynchronous,{mode:'read'},{append:{}});
    assert.equal(malformed.error.code,'invalid_response');
    assert.equal(malformed.error.effectState,'unknown');
    assert.equal(malformed.calls.length,2);
  });
}

test('没有历史接口先失败，不读取或发送复制',async()=>{
  for(const main of [asyncMain,syncMain]){
    let called=false;
    const ctx={input:{mode:'copy'},clipboard:{readData:()=>{called=true;}},
      desktop:{copySelectionData:()=>{called=true;}}};
    let error;
    try{await main(ctx);}catch(value){error=value;}
    assert.equal(error.code,'history_unavailable');
    assert.equal(error.effectState,'not_started');
    assert.equal(called,false);
  }
});

test('等待策略在开始前快照，后续对象改动不允许旧数据',()=>{
  const input={mode:'copy',options:{timeoutMs:400,pollIntervalMs:10,settleMs:0,allowUnchanged:false}};
  const task=prepare(input);
  input.options.allowUnchanged=true;
  assert.equal(task.allowUnchanged,false);
  assert.equal(task.options.allowUnchanged,false);
});

test('共享数据SDK与独立历史工厂可组合为同步或Promise桥接',async()=>{
  const root=new URL('../../../crates/omnitrigger-actions/runtime/',import.meta.url);
  const desktop=runInNewContext(readFileSync(new URL('desktop-api.js',root),'utf8')+';createDesktopApi');
  const history=runInNewContext(readFileSync(new URL('clipboard-history-api.js',root),'utf8')+';createClipboardHistoryApi');
  for(const asynchronous of [false,true]){
    const calls=[];
    const bridge=(method,params)=>{
      calls.push([method,params]);
      const value=method==='selection.copy_data'?{kind:'text',text:secret,sequenceNumber:0,changed:true}:receipt;
      return asynchronous?Promise.resolve(value):value;
    };
    const api=desktop(bridge);
    const ctx={...api,input:{mode:'copy'},clipboard:{...api.clipboard,history:history(bridge)}};
    const result=asynchronous?await asyncMain(ctx):syncMain(ctx);
    assert.equal(result.saved,true);
    assert.deepEqual(calls.map(call=>call[0]),['selection.copy_data','history.append']);
  }
});

test('两入口清单和read/copy调用明确，不创建监听器',()=>{
  const manifest=JSON.parse(readFileSync(new URL('action.json',import.meta.url),'utf8'));
  assert.deepEqual(manifest.actions.map(action=>action.runtime),['node','script']);
  for(const name of ['call.json','call-script.json','copy-call.json']){
    const call=JSON.parse(readFileSync(new URL(name,import.meta.url),'utf8'));
    const action=manifest.actions.find(action=>action.id===call.action_id);
    assert.equal(call.package_id,manifest.id);
    assert.equal(call.action_version,action.version);
    assert.equal(action.single_instance,true);
  }
});
