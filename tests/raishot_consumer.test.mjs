import {whitespaceCodeUnits} from '../examples/actions/win32-edit-line-cleanup/lib/dotnet-whitespace.mjs';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import assert from 'node:assert/strict';
import nodeMain from '../examples/actions/raishot/node.mjs';
import scriptMain, {main as namedScriptMain} from '../examples/actions/raishot/script.mjs';
import {configuration,absoluteDirectory,packageResult} from '../examples/actions/raishot/lib.mjs';

const PROGRAM='E:\\Synthetic System Tools\\powershell.exe';
const report={directory:'C:\\Owned RaiShot\\0.4.18',exe_path:'C:\\Owned RaiShot\\0.4.18\\raishot.exe',version:'0.4.18',verification:'full',source:'existing',archive_authenticated:true,package_name:'rustshot'};
function fixture(kind,{resident=null,input={},reply={queued:true,screenshotCompleted:false},pkg=report,error=null,cancelAfterPrepare=false}={}) {
  const events=[],signal={aborted:false};
  const wrap=fn=>(...args)=>kind==='Node'?Promise.resolve().then(()=>fn(...args)):fn(...args);
  const ctx={input:{powerShellProgram:PROGRAM,...input},packageRoot:'C:\\Owned Consumer',signal,
    desktop:{findRaiShotResident:wrap(()=>{events.push(['find']);return resident;}),requestRaiShotRegion:wrap(snapshot=>{events.push(['region',snapshot]);if(error)throw error;return reply;})},
    process:{capture:wrap((program,args,options)=>{events.push(['prepare',program,args,options]);if(cancelAfterPrepare)signal.aborted=true;return {code:0,stdout:JSON.stringify(pkg),stderr:''};}),
      start:wrap((...args)=>{events.push(['start',...args]);return {pid:4123};})}};
  return {events,ctx,run:()=>kind==='Node'?nodeMain(ctx):scriptMain(ctx)};
}
for(const kind of ['Node','Script']) {
  test(`${kind} 常驻 region 仅查询与投递，不访问依赖或再启动`,async()=>{
    const snapshot=Object.freeze({id:1}),f=fixture(kind,{resident:snapshot});
    assert.deepEqual(await f.run(),{status:'queued',mode:'region',queued:true,screenshotCompleted:false});
    assert.deepEqual(f.events,[['find'],['region',snapshot]]);
  });
  test(`${kind} 常驻 tray 仅查询，无依赖与启动`,async()=>{
    const f=fixture(kind,{resident:{id:1},input:{launch_mode:'tray'}});
    assert.equal((await f.run()).status,'resident');assert.deepEqual(f.events,[['find']]);
  });
  test(`${kind} verify 跳过常驻查询，完整核验后不启动`,async()=>{
    const f=fixture(kind,{resident:{id:1},input:{launch_mode:'verify'}}),r=await f.run();
    assert.equal(r.status,'verified');assert.equal(r.verification,'full');assert.deepEqual(f.events.map(e=>e[0]),['prepare']);
    assert.equal(f.events[0][2].at(-1),'verify');
  });
  for(const mode of ['region','tray']) test(`${kind} 无常驻 ${mode} 按指定启动契约调用`,async()=>{
    const f=fixture(kind,{input:{launch_mode:mode}});assert.deepEqual(await f.run(),{status:'launched',mode,pid:4123,screenshotCompleted:false});
    assert.deepEqual(f.events.map(e=>e[0]),['find','prepare','start']);
    assert.deepEqual(f.events.at(-1),kind==='Node'?['start',report.exe_path,[`--${mode}`],{cwd:report.directory,windowsHide:false}]:['start',report.exe_path,[`--${mode}`]]);
    assert.deepEqual(f.events[1].slice(1),[PROGRAM,['-NoLogo','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File','C:\\Owned Consumer\\package-cli.ps1','-Operation','prepare'],{timeoutMs:60000,allowNonZero:true,cwd:'C:\\Owned Consumer'}]);
  });
  test(`${kind} 显式参数优先，显式依赖目录只传结构化参数`,async()=>{
    const f=fixture(kind,{input:{quicker_in_param:' VERIFY ',launch_mode:'tray',packageDirectory:'D:\\合成 依赖\\0.4.18'}});
    assert.equal((await f.run()).status,'verified');assert.deepEqual(f.events[0][2].slice(-2),['-PackageDirectory','D:\\合成 依赖\\0.4.18']);
  });
  test(`${kind} 无效模式在任何 API 前拒绝`,async()=>{
    for(const input of [{launch_mode:'bad'},{launch_mode:'  '},{launch_mode:0},{quicker_in_param:12}]) {
      const f=fixture(kind,{input});await assert.rejects(async()=>f.run(),e=>e.code==='invalid_input');assert.equal(f.events.length,0);
    }
  });
  test(`${kind} 常驻身份变化保持错误与单次投递，不退回启动`,async()=>{
    const error=Object.assign(new Error('目标变化'),{code:'raishot_resident_changed',effectState:'not_started'});
    const f=fixture(kind,{resident:{id:1},error});await assert.rejects(async()=>f.run(),e=>e===error);assert.deepEqual(f.events.map(e=>e[0]),['find','region']);
  });
  test(`${kind} 投递未知结果不重复投递或启动`,async()=>{
    const error=Object.assign(new Error('传输未确认'),{code:'timeout',effectState:'unknown',retryAllowed:false});
    const f=fixture(kind,{resident:{id:1},error});await assert.rejects(async()=>f.run(),e=>e===error);assert.equal(f.events.length,2);
  });
  test(`${kind} 校验失败或过旧版本不启动`,async()=>{
    for(const pkg of [{...report,version:'0.4.15'},{...report,verification:'size'},{...report,exe_path:'C:\\other.exe'},{...report,package_name:'unknown'}]) {
      const f=fixture(kind,{input:{launch_mode:'verify'},pkg});await assert.rejects(async()=>f.run());assert.deepEqual(f.events.map(e=>e[0]),['prepare']);
    }
  });
}
test('Node 准备完成后取消，不启动程序',async()=>{
  const f=fixture('Node',{cancelAfterPrepare:true});await assert.rejects(f.run,e=>e.code==='cancelled');assert.deepEqual(f.events.map(e=>e[0]),['find','prepare']);
});
test('配置默认区域截图，拒绝云盘、相对路径、设备路径和路径穿越',()=>{
  assert.deepEqual(configuration({}),{mode:'region',directory:''});
  assert.equal(configuration({quicker_in_param:' ',launch_mode:'tray'}).mode,'tray');
  for(const path of ['relative','C:\\OneDrive - test\\a','C:\\Dropbox\\a','C:\\iCloudDrive\\a','C:\\x\\..\\a','\\\\server\\a','C:\\x.\\a','C:\\x:stream']) assert.throws(()=>absoluteDirectory(path));
});
test('兼容两种已支持包名，保留来源与版本',()=>{
  for(const package_name of ['rustshot','cea.rustshot']) {
    const r=packageResult({code:0,stdout:JSON.stringify({...report,package_name,version:'0.4.19',archive_authenticated:false}),stderr:''},'verify');
    assert.equal(r.package_name,package_name);assert.equal(r.version,'0.4.19');assert.equal(r.archive_authenticated,false);
  }
});
test('异常 helper 回执不泄露正文或当作成功',()=>{
  for(const result of [{code:1,stdout:'PRIVATE',stderr:'PRIVATE'},{code:0,stdout:'PRIVATE',stderr:''},{code:0,stdout:'null',stderr:''},{code:0,stdout:'x'.repeat(8193),stderr:''}]) assert.throws(()=>packageResult(result,'verify'),e=>!e.message.includes('PRIVATE'));
});

test('Script 保留具名 main 且与默认入口一致',()=>{assert.equal(namedScriptMain,scriptMain);});

test('真实 capture 校验器复现旧拒绝、接受显式解释器；常驻快速路径无需解释器',async()=>{
  const sdk=readFileSync(new URL('../crates/omnitrigger-actions/runtime/process-capture-api.js',import.meta.url),'utf8');
  const create=vm.runInNewContext(sdk+'\ncreateProcessCaptureApi');
  const packageRoot='F:\\Synthetic Actions\\RaiShot';
  const directory='G:\\Synthetic RaiShot\\0.4.18';
  const powerShellProgram='E:\\Synthetic System Tools\\powershell.exe';
  const report={directory,exe_path:`${directory}\\raishot.exe`,version:'0.4.18',verification:'full',source:'existing',archive_authenticated:true,package_name:'rustshot'};
  assert.equal(namedScriptMain,scriptMain);
  for(const asyncMode of [true,false]) {
    const calls=[];
    const capture=create(params=>{calls.push(JSON.parse(JSON.stringify(params)));return {code:0,stdout:JSON.stringify(report),stderr:''};},asyncMode).capture;
    const old={program:'powershell.exe',args:[],options:{timeoutMs:60000,allowNonZero:true,cwd:packageRoot}};
    await assert.rejects(async()=>capture(old.program,old.args,old.options),error=>error.code==='invalid_input'&&error.effectState==='not_started'&&error.method==='process.capture');
    assert.equal(calls.length,0,'旧相对程序名必须在派发前拒绝');
    const main=asyncMode?nodeMain:scriptMain;
    const noDesktop={findRaiShotResident(){throw new Error('verify 不得查询常驻');}};
    const ctx={input:{launch_mode:'verify',packageDirectory:directory},packageRoot,signal:{aborted:false},desktop:noDesktop,process:{capture,start(){throw new Error('verify 不得启动 RaiShot');}}};
    await assert.rejects(async()=>main(ctx),error=>error.code==='invalid_input'&&error.effectState==='not_started'&&error.message.includes('powerShellProgram'));
    assert.equal(calls.length,0);
    ctx.input.powerShellProgram=powerShellProgram;
    const output=await main(ctx);
    assert.equal(output.status,'verified');
    assert.equal(calls.length,1);
    assert.deepEqual(calls[0],{program:powerShellProgram,args:['-NoLogo','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',`${packageRoot}\\package-cli.ps1`,'-Operation','verify','-PackageDirectory',directory],timeout_ms:60000,allow_non_zero:true,cwd:packageRoot});
    for(const mode of ['region','tray']) {
      let regionRequests=0;
      const warm={input:{launch_mode:mode},packageRoot,signal:{aborted:false},desktop:{findRaiShotResident:()=>({ownedFixture:true}),requestRaiShotRegion:()=>{regionRequests++;return {queued:true,screenshotCompleted:false};}},process:{capture(){throw new Error('常驻快速路径不得访问 helper');},start(){throw new Error('常驻快速路径不得启动程序');}}};
      assert.equal((await main(warm)).status,mode==='region'?'queued':'resident');
      assert.equal(regionRequests,mode==='region'?1:0);
    }
  }
});

test('按C#空白集合区分显式参数回退、null默认与Trim后空模式',()=>{
  assert.equal(configuration({launch_mode:null}).mode,'region');
  assert.equal(configuration({quicker_in_param:' ',launch_mode:'tray'}).mode,'tray');
  assert.throws(()=>configuration({launch_mode:' '}),e=>e.code==='invalid_input');
  assert.throws(()=>configuration({quicker_in_param:' ',launch_mode:' '}),e=>e.code==='invalid_input');
  for(const unit of whitespaceCodeUnits) {
    const space=String.fromCharCode(unit);
    assert.equal(configuration({quicker_in_param:space,launch_mode:'tray'}).mode,'tray');
    assert.equal(configuration({launch_mode:space+' VERIFY '+space}).mode,'verify');
    assert.throws(()=>configuration({launch_mode:space}),e=>e.code==='invalid_input');
  }
  assert.throws(()=>configuration({quicker_in_param:'\uFEFF',launch_mode:'tray'}),e=>e.code==='invalid_input');
  assert.throws(()=>configuration({launch_mode:'\uFEFFregion\uFEFF'}),e=>e.code==='invalid_input');
});

test('消费者保留两原入口并包含文件结果入口与配套声明',()=>{
  const manifest=JSON.parse(readFileSync(new URL('../examples/actions/raishot/action.json',import.meta.url),'utf8'));
  assert.equal(manifest.actions.length,3);
  assert.deepEqual(manifest.actions.map(action=>action.id),['node','script','result-script']);
  for(const action of manifest.actions) {
    assert.equal(action.version,1);
    assert(readFileSync(new URL('../examples/actions/raishot/'+action.entry,import.meta.url)).length>0);
  }
  const entry=manifest.actions[2];assert.equal(entry.runtime,'script');assert.equal(entry.entry,'result-script.mjs');assert.equal(entry.single_instance,true);
  const config=JSON.parse(readFileSync(new URL('../examples/actions/raishot/jsconfig.json',import.meta.url),'utf8'));
  assert(config.files.includes('result-script.mjs'));
  for(const declaration of ['types/api.d.ts','sdk/api.d.ts']) {
    const source=readFileSync(new URL('../examples/actions/raishot/'+declaration,import.meta.url),'utf8');
    for(const method of ['captureRaiShotResult','releaseRaiShotResult'])assert(source.includes(method));
  }
});
