// 共用SDK的纯内存合同与严格类型回归；不启动应用或平台宿主。
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,mkdirSync,mkdtempSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import vm from 'node:vm';
const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
export async function checkActionHint({typescript:ts,temporaryRoot=join(repo,'.tmp'),
  runtimeRoot=join(repo,'crates/omnitrigger-actions/runtime')}={}) {
  assert(ts?.createProgram,'需要当前TypeScript');
  mkdirSync(temporaryRoot,{recursive:true});
  const root=mkdtempSync(join(temporaryRoot,'action-hint-'));
  const create=vm.runInNewContext(readFileSync(join(runtimeRoot,'desktop-api.js'),'utf8')+'\ncreateDesktopApi;');
  let runtimeCases=0;
  for(const asynchronous of [false,true]) {
    const calls=[];let reply={updated:true};let failure;
    const bridge=(method,params)=>{calls.push({method,params});if(failure)throw failure;return reply;};
    const api=create(asynchronous?async(...args)=>bridge(...args):bridge);
    for(const text of ['n10','pre','beta','🙂'.repeat(8)]) {
      const result=api.ui.setActionHint({text});assert.equal(typeof result?.then,asynchronous?'function':'undefined');
      assert.equal((await result).updated,true);assert.equal(calls.at(-1).method,'ui.action_hint');
      assert.equal(JSON.stringify(calls.at(-1).params),JSON.stringify({text}));runtimeCases++;
    }
    const getter={get text(){throw new Error('不得读取访问器');}};
    for(const options of [undefined,null,[],{},getter,{text:''},{text:' n10'},{text:'n10\n'},
      {text:'x'.repeat(17)},{text:'🙂'.repeat(9)},{text:'\ud800'},{text:'\udc00'},
      {text:'a\u0085b'},{text:'a\u2028b'},{text:'\ufeffn10'},{text:'n10',scope:{}},{text:'n10',color:'green'}]) {
      const before=calls.length;let error;try{await api.ui.setActionHint(options);}catch(value){error=value;}
      assert.equal(error?.code,'invalid_input');assert.equal(error.method,'ui.action_hint');
      assert.equal(error.effectState,'not_started');assert.equal(calls.length,before);runtimeCases++;
    }
    for(const value of [null,{},[],{updated:false},{updated:true,rendered:true},true]) {
      reply=value;let error;try{await api.ui.setActionHint({text:'n10'});}catch(value){error=value;}
      assert.equal(error?.code,'invalid_response');assert.equal(error.effectState,'unknown');runtimeCases++;
    }
    failure=Object.assign(new Error('合成范围过期'),{code:'ui_hint_scope_expired',effectState:'not_started',method:'ui.action_hint'});
    let error;try{await api.ui.setActionHint({text:'n10'});}catch(value){error=value;}
    assert.equal(error,failure);runtimeCases++;
  }
  const files=[];
  for(const kind of ['node','script']) {
    const path=join(root,kind+'.ts');const declaration=join(runtimeRoot,kind==='node'?'api.d.ts':'script-types.d.ts').replaceAll('\\','/');
    const header=kind==='node'?`import type {ActionContext} from ${JSON.stringify(declaration)};`:`/// <reference path=${JSON.stringify(declaration)} />`;
    writeFileSync(path,header+'\nasync function check(ctx:'+(kind==='node'?'ActionContext':'ScriptContext')+`){
      const receipt:${kind==='node'?'Promise<{updated:true}>':'{updated:true}'}=ctx.ui.setActionHint({text:'n10'});
      // @ts-expect-error 不接收作用域
      ctx.ui.setActionHint({text:'n10',scope:{}});
      // @ts-expect-error 必须是字符串
      ctx.ui.setActionHint({text:1});
      // @ts-expect-error 不接收样式
      ctx.ui.setActionHint({text:'n10',color:'green'});
      // @ts-expect-error 必须带参数
      ctx.ui.setActionHint();
      // @ts-expect-error 同步/异步合同不可互换
      receipt.${kind==='node'?'updated':'then'};
    }\n`);files.push(path);
  }
  const diagnostics=ts.getPreEmitDiagnostics(ts.createProgram(files,{strict:true,noEmit:true,types:[],target:ts.ScriptTarget.ES2022,lib:['lib.es2022.d.ts'],skipLibCheck:false}));
  assert.equal(diagnostics.length,0,ts.formatDiagnostics(diagnostics,{getCanonicalFileName:p=>p,getCurrentDirectory:()=>repo,getNewLine:()=> '\n'}));
  return {passed:true,runtimeCases,typePrograms:2,noEmit:true,compilerVersion:ts.version,root};
}
