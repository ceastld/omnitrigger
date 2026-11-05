// TypeScript严格noEmit，虚拟源码；不创建工程、不导入/执行作者脚本或宿主。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import {dirname,resolve,sep} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
const require=createRequire(new URL('../tools/script-sdk-check/package.json',import.meta.url));
const ts=require('typescript');const tsLib=dirname(require.resolve('typescript'));
const out=dirname(fileURLToPath(import.meta.url));const virtual=resolve(out,'.virtual-types');
const files={
  'api.d.ts':readFileSync(new URL('../crates/omnitrigger-actions/runtime/api.d.ts',import.meta.url),'utf8'),
  'script-types.d.ts':readFileSync(new URL('../crates/omnitrigger-actions/runtime/script-types.d.ts',import.meta.url),'utf8'),
  'clipboard-history-api.d.ts':readFileSync(new URL('../crates/omnitrigger-actions/runtime/clipboard-history-api.d.ts',import.meta.url),'utf8'),
};

function check(text,source=files){
  const cases=typeof text==='string'?{'case.ts':text}:text;
  const contents=new Map(Object.entries({...source,...cases}).map(([name,value])=>[resolve(virtual,name),value]));
  const options={target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,moduleResolution:ts.ModuleResolutionKind.Bundler,
    strict:true,exactOptionalPropertyTypes:true,noUncheckedIndexedAccess:true,noEmit:true,skipLibCheck:false,types:[],
    lib:['lib.es2022.d.ts','lib.dom.d.ts']};
  const host=ts.createCompilerHost(options);const originalRead=host.readFile,originalExists=host.fileExists;
  const library=file=>resolve(file).startsWith(tsLib+sep);
  host.readFile=file=>contents.get(resolve(file))??(library(file)?originalRead(file):undefined);
  host.fileExists=file=>contents.has(resolve(file))||(library(file)&&originalExists(file));
  host.directoryExists=path=>resolve(path)===virtual||resolve(path)===tsLib||resolve(path).startsWith(tsLib+sep);
  host.getDirectories=()=>[];host.getCurrentDirectory=()=>virtual;host.realpath=file=>file;
  host.getSourceFile=(file,language)=>{const value=host.readFile(file);return value===undefined?undefined:ts.createSourceFile(file,value,language,true);};
  const program=ts.createProgram([...Object.keys(cases).map(name=>resolve(virtual,name)),resolve(virtual,'script-types.d.ts')],options,host);
  return ts.getPreEmitDiagnostics(program);
}
const prefix=`import type {ActionContext,BrightnessStepResult,BrightnessStepMember,BrightnessStepWmi} from './api';\ndeclare const ctx:ActionContext;\ndeclare const sync:ScriptContext;\n`;
const results=[];
test('严格TypeScript正例覆盖两入口、显式策略及可安全收窄的逐项回执',()=>{
  const positive=[
    `const p:Promise<BrightnessStepResult>=ctx.desktop.stepBrightness({direction:'up',strategy:'physical'});
     async function consume(){const r=await p;if(r.physical&&r.physical.errorCode===null)for(const m of r.physical.members){if(m.status==='confirmed')m.currentRaw.toFixed();}
       if(r.wmi?.status==='set')r.wmi.requestedPercent.toFixed();}`,
    `const r:ScriptBrightnessStepResult=sync.desktop.stepBrightness({direction:'down',strategy:'wmi_first'});
     const noWrite:BrightnessStepResult={strategy:'physical',effectState:'not_started',physical:{errorCode:null,members:[{index:0,status:'unchanged',previousRaw:40,requestedRaw:40,currentRaw:40,errorCode:null}]},wmi:null};
     if(r.effectState==='not_started'&&r.physical&&r.physical.errorCode===null)for(const m of r.physical.members){if(m.status==='unchanged')m.currentRaw.toFixed();}`
  ];
  for(const [index,value]of positive.entries()){const diagnostics=check(prefix+value);assert.deepEqual(diagnostics.map(d=>[d.code,ts.flattenDiagnosticMessageText(d.messageText,' ')]),[]);results.push({kind:'positive',index,diagnostics:0});}
});
test('严格TypeScript反例分别拒绝缺参、错值、错误异步形状和不一致回执',()=>{
  const negatives={
    missing_strategy:`ctx.desktop.stepBrightness({direction:'up'});`,
    missing_direction:`ctx.desktop.stepBrightness({strategy:'physical'});`,
    bad_direction:`ctx.desktop.stepBrightness({direction:'UP',strategy:'physical'});`,
    bad_strategy:`ctx.desktop.stepBrightness({direction:'up',strategy:'auto'});`,
    extra_field:`ctx.desktop.stepBrightness({direction:'up',strategy:'physical',retry:true});`,
    absent_argument:`ctx.desktop.stepBrightness();`,
    array_argument:`ctx.desktop.stepBrightness(['up','physical']);`,
    null_argument:`ctx.desktop.stepBrightness(null);`,
    bad_sync_strategy:`sync.desktop.stepBrightness({direction:'up',strategy:'auto'});`,
    sync_missing_strategy:`sync.desktop.stepBrightness({direction:'down'});`,
    sync_extra_field:`sync.desktop.stepBrightness({direction:'up',strategy:'wmi_first',point:[0,0]});`,
    async_is_not_sync:`const r:BrightnessStepResult=ctx.desktop.stepBrightness({direction:'up',strategy:'physical'});`,
    sync_is_not_promise:`const p:Promise<ScriptBrightnessStepResult>=sync.desktop.stepBrightness({direction:'down',strategy:'physical'});`,
    confirmed_null:`const m:BrightnessStepMember={index:0,status:'confirmed',previousRaw:40,requestedRaw:45,currentRaw:null,errorCode:null};`,
    read_rejected_value:`const m:BrightnessStepMember={index:0,status:'read_rejected',previousRaw:null,requestedRaw:null,currentRaw:45,errorCode:'brightness_failed'};`,
    confirmed_error:`const m:BrightnessStepMember={index:0,status:'confirmed',previousRaw:40,requestedRaw:45,currentRaw:45,errorCode:'brightness_unknown'};`,
    wmi_set_null:`const w:BrightnessStepWmi={status:'set',previousPercent:40,requestedPercent:null,currentPercent:60,errorCode:null};`,
    wmi_zero_target:`const w:BrightnessStepWmi={status:'zero_skipped',previousPercent:2,requestedPercent:0,currentPercent:2,errorCode:null};`,
    wmi_failed_current:`const w:BrightnessStepWmi={status:'failed',previousPercent:null,requestedPercent:null,currentPercent:60,errorCode:'brightness_unknown'};`,
    not_started_confirmed:`const r:BrightnessStepResult={strategy:'physical',effectState:'not_started',physical:{errorCode:null,members:[{index:0,status:'confirmed',previousRaw:40,requestedRaw:45,currentRaw:45,errorCode:null}]},wmi:null};`,
    not_started_unknown:`const r:BrightnessStepResult={strategy:'wmi_first',effectState:'not_started',physical:null,wmi:{status:'failed',previousPercent:null,requestedPercent:null,currentPercent:null,errorCode:'brightness_unknown'}};`,
    physical_has_wmi:`const r:BrightnessStepResult={strategy:'physical',effectState:'unknown',physical:{members:[],errorCode:'monitor_unavailable'},wmi:{status:'failed',previousPercent:null,requestedPercent:null,currentPercent:null,errorCode:'brightness_failed'}};`,
    member_identifier:`const m:BrightnessStepMember={index:0,status:'confirmed',previousRaw:40,requestedRaw:45,currentRaw:45,errorCode:null,deviceId:'synthetic'};`,
    invalid_effect:`const r:BrightnessStepResult={strategy:'wmi_first',effectState:'completed',physical:null,wmi:{status:'zero_skipped',previousPercent:2,requestedPercent:null,currentPercent:2,errorCode:null}};`,
  };
  const allDiagnostics=check(Object.fromEntries(Object.entries(negatives).map(([name,value])=>[name+'.ts',prefix+value])));
  for(const name of Object.keys(negatives)){const diagnostics=allDiagnostics.filter(d=>d.file&&resolve(d.file.fileName)===resolve(virtual,name+'.ts'));assert(diagnostics.length>0,name);results.push({kind:'negative',name,diagnosticCodes:[...new Set(diagnostics.map(d=>d.code))]});}
});
