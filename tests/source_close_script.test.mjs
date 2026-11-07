import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {main} from '../examples/actions/keyboard-sequence/source-close-script.mjs';
const known={processName:'Acrobat.exe',processId:42,isForeground:true,windowRole:'application',title:'不应进入回执'};
function fixture(spec={}){
  const calls=[];const ctx={input:Object.hasOwn(spec,'input')?spec.input:{},desktop:{
    target(){calls.push({method:'target'});if(spec.targetError)throw spec.targetError;return Object.hasOwn(spec,'target')?spec.target:known;},
    sendShortcut(...args){calls.push({method:'sendShortcut',args});if(spec.sendError)throw spec.sendError;return Object.hasOwn(spec,'result')?spec.result:{submitted:true};},
  }};
  for(const key of ['browser','clipboard','process','delay','open','fs'])Object.defineProperty(ctx,key,{get(){assert.fail(`禁止访问${key}`);}});
  for(const key of ['targetContext','captureForegroundTarget','sendShortcutTo','sendText','windowAction','sendKeySequence'])Object.defineProperty(ctx.desktop,key,{get(){assert.fail(`禁止替代原目标快捷键API：${key}`);}});
  return{ctx,calls};
}
for(const [name,branch,keys] of [
  ['Acrobat.exe','acrobat-ctrl-w',[162,87]],
  ['chrome.exe','chrome-alt-f4',[164,115]],
  ['notepad.exe','other-alt-f4',[164,115]],
  ['msedge.exe','other-alt-f4',[164,115]],
  ['AcroRd32.exe','other-alt-f4',[164,115]],
  ['acrobat.exe','other-alt-f4',[164,115]],
  ['Acrobat.EXE','acrobat-ctrl-w',[162,87]],
  ['Chrome.exe','other-alt-f4',[164,115]],
  ['中文应用.exe','other-alt-f4',[164,115]],
  ['my app.exe','other-alt-f4',[164,115]],
])test(`来源关闭进程分支：${name}`,()=>{
  const f=fixture({target:{...known,processName:name}});const result=main(f.ctx);
  assert.equal(typeof result?.then,'undefined');assert.deepEqual(result,{inputSubmitted:true,branch});
  assert.deepEqual(f.calls,[{method:'target'},{method:'sendShortcut',args:[keys]}]);
});
test('未知进程身份不落入其他进程关闭分支',()=>{
  const invalid=[null,undefined,{},...['','Acrobat','.exe','Acrobat.exe ','C:/Acrobat.exe','C:\\Acrobat.exe','bad\n.exe'].map(processName=>({...known,processName})),
    ...[undefined,0,-1,1.5,4294967296].map(processId=>({...known,processId}))];
  for(const target of invalid){const f=fixture({target});assert.throws(()=>main(f.ctx),e=>e.code==='target_process_unknown'&&e.effectState==='not_started');assert.deepEqual(f.calls,[{method:'target'}]);}
});
test('原目标失去前台、桌面、任务栏或未知窗口角色均不派发',()=>{
  for(const target of [{...known,isForeground:false},{...known,isForeground:undefined},...['desktop','taskbar','unknown',undefined].map(windowRole=>({...known,windowRole}))]){
    const f=fixture({target});assert.throws(()=>main(f.ctx),e=>e.code==='target_changed'&&e.effectState==='not_started');assert.deepEqual(f.calls,[{method:'target'}]);
  }
});
test('固定来源参数不能被改写，校验先于目标查询',()=>{
  for(const input of [null,undefined,[],{processName:'chrome.exe'},{allowedProcesses:['chrome']},{keys:[164,115]},{[Symbol('hidden')]:true}]){
    const f=fixture({input});assert.throws(()=>main(f.ctx),e=>e.code==='invalid_input'&&e.effectState==='not_started');assert.deepEqual(f.calls,[]);
  }
});
test('原目标查询错误保持结构且不回退重抓前台',()=>{
  const error=Object.assign(new Error('身份已变'),{code:'target_changed',method:'target.info',effectState:'not_started'});
  const f=fixture({targetError:error});assert.throws(()=>main(f.ctx),e=>e===error);assert.deepEqual(f.calls,[{method:'target'}]);
});
for(const effectState of ['not_started','unknown'])test(`发送失败${effectState}不重试或换API`,()=>{
  const error=Object.assign(new Error('未完成'),{code:'input_failed',method:'keyboard.shortcut',effectState});const f=fixture({sendError:error});
  assert.throws(()=>main(f.ctx),e=>e===error);assert.deepEqual(f.calls,[{method:'target'},{method:'sendShortcut',args:[[162,87]]}]);
});
test('发送回执未知时保守停止，不声称关闭成功',()=>{
  for(const result of [null,undefined,{},false,{submitted:false},Promise.resolve({submitted:true})]){
    const f=fixture({result});assert.throws(()=>main(f.ctx),e=>e.code==='input_unconfirmed'&&e.effectState==='unknown');assert.equal(f.calls.filter(c=>c.method==='sendShortcut').length,1);
  }
});
test('包注册、同步入口、调用模板与作者工程一致',async()=>{
  const base=new URL('../examples/actions/keyboard-sequence/',import.meta.url);
  const manifest=JSON.parse(await readFile(new URL('action.json',base),'utf8'));const entry=manifest.actions.find(a=>a.id==='source-close-script');
  assert.deepEqual(entry,{id:'source-close-script',title:'按原目标进程发送关闭快捷键',version:1,runtime:'script',entry:'source-close-script.mjs'});
  const call=JSON.parse(await readFile(new URL('source-close-script-call.json',base),'utf8'));assert.deepEqual(call,{package_id:manifest.id,action_id:entry.id,action_version:entry.version,input:{}});
  const project=JSON.parse(await readFile(new URL('jsconfig.json',base),'utf8'));
  assert.ok(project.files.includes(entry.entry));assert.ok(project.files.includes('sdk.d.ts'));
  for(const name of project.files){
    const file=new URL(name,base);assert.ok(file.href.startsWith(base.href),'作者工程只引用包内文件');await readFile(file);
  }
});

const code=await readFile(new URL('../crates/omnitrigger-actions/runtime/desktop-api.js',import.meta.url),'utf8');
const createDesktopApi=Function(code+'\nreturn createDesktopApi;')();
for(const [processName,keys] of [['Acrobat.exe',[162,87]],['chrome.exe',[164,115]],['notepad.exe',[164,115]]])test(`实际共享desktop工厂内存接线：${processName}`,()=>{
  const wire=[];const {desktop}=createDesktopApi((method,params)=>{
    wire.push({method,params});
    if(method==='target.info')return{processName,processId:42,isForeground:true,windowRole:'application',title:'私有合成窗口'};
    assert.equal(method,'keyboard.shortcut');return{submitted:true};
  });
  const result=main({input:{},desktop});assert.equal(result.inputSubmitted,true);
  assert.deepEqual(wire,[{method:'target.info',params:{}},{method:'keyboard.shortcut',params:{keys}}]);
});
