// 对既有JavaScript工程做入口与依赖的 noEmit 类型检查；不创建工程、不导入或执行作者代码。
import {spawnSync} from 'node:child_process';
import {openSync,closeSync,fstatSync,lstatSync,readdirSync,readSync} from 'node:fs';
import {dirname,extname,isAbsolute,join,relative,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import ts from 'typescript';
import {analyzeScriptModules} from './analyze-script-modules.mjs';
import {analyzeScriptEntry} from './analyze-script-entry.mjs';
import {analyzeNodeEntries} from './check-node-entry.cjs';

class ProjectCheckError extends Error {
  constructor(code,location) { super('脚本工程检查未完成；请检查错误代码。'); this.code=code; this.location=location; }
}
const fail=(code,location)=>{throw new ProjectCheckError(code,location);};
function within(root,path) {
  const name=relative(root,path);
  return name==='' || !isAbsolute(name)&&name!=='..'&&!name.startsWith('..\\')&&!name.startsWith('../');
}
function ancestors(path) {
  const items=[path];
  while(dirname(items[0])!==items[0])items.unshift(dirname(items[0]));
  return items;
}
function metadataGuard() {
  const checked=new Set();
  return paths=>{
    const pending=new Set();
    for(const path of paths)for(const item of ancestors(resolve(path))) {
      const name=item.slice(dirname(item).length).replace(/^[\\/]/,'').toLowerCase();
      if(name.startsWith('onedrive')||name.startsWith('icloud')||name==='dropbox')fail('project_cloud_path');
      const info=lstatSync(item);
      if(info.isSymbolicLink())fail('project_link_path');
      if(item===resolve(path)&&info.isFile()&&info.nlink>1)fail('project_link_path');
      if(!checked.has(item))pending.add(item);
    }
    if(process.platform==='win32') {
      const command="$ErrorActionPreference='Stop'; try { $paths=ConvertFrom-Json $env:OMNITRIGGER_SDK_METADATA_PATHS; foreach($p in $paths) { $item=Get-Item -LiteralPath $p -Force; if(([long]$item.Attributes -band 0x441400) -ne 0) { exit 7 } }; exit 0 } catch { exit 8 }";
      const values=[...pending];
      for(let index=0;index<values.length;index+=64) {
        const result=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from(command,'utf16le').toString('base64')],
          {windowsHide:true,encoding:'utf8',timeout:15000,maxBuffer:4096,
            env:{...process.env,OMNITRIGGER_SDK_METADATA_PATHS:JSON.stringify(values.slice(index,index+64))}});
        if(result.error||result.status!==0)fail('project_metadata_unavailable');
      }
    }
    for(const item of pending)checked.add(item);
  };
}

export function checkProjectTypes(projectDirectory) {
  if(typeof projectDirectory!=='string'||!projectDirectory)fail('project_directory_required');
  const root=resolve(projectDirectory);
  if(process.platform==='win32'&&!/^(?:[A-Za-z]:\\|\\\\\?\\[A-Za-z]:\\)/.test(root))fail('project_local_directory_required');
  const guard=metadataGuard();
  const library=dirname(ts.getDefaultLibFilePath({target:ts.ScriptTarget.ES2020}));
  guard([root,library]);
  if(!lstatSync(root).isDirectory())fail('project_directory_required');
  // 只列已定位的编译器标准库名称；工程内不递归搜索，依赖由静态引用按需读取。
  const libraries=readdirSync(library).filter(name=>/^lib\..*\.d\.ts$/.test(name)).map(name=>join(library,name));
  guard(libraries);
  const loaded=new Map();
  const sourceSnapshots=new Map();
  let total=0;
  let configurationReads;
  function read(path,limit) {
    path=resolve(path);
    if(!within(root,path)&&!within(library,path))return undefined;
    guard([path]);
    const handle=openSync(path,'r');
    try {
      const info=fstatSync(handle);
      if(!info.isFile()||info.size>limit)fail('project_file_limit');
      const bytes=Buffer.alloc(limit+1);
      let count=0;
      while(count<bytes.length) {
        const next=readSync(handle,bytes,count,bytes.length-count,null);
        if(next===0)break;
        count+=next;
      }
      if(count>limit)fail('project_file_limit');
      if(within(root,path)&&!loaded.has(path)) {
        total+=count;loaded.set(path,count);
        if(loaded.size>256||total>8*1024*1024)fail('project_total_limit');
      }
      let text;
      try{text=new TextDecoder('utf-8',{fatal:true}).decode(bytes.subarray(0,count));}
      catch {fail('project_utf8_invalid',within(root,path)?{file:relative(root,path).replaceAll('\\','/')}:undefined);}
      if(within(root,path)&&['.js','.mjs'].includes(extname(path)))sourceSnapshots.set(path,text);
      if(within(root,path))configurationReads?.add(path);
      return text;
    } finally {closeSync(handle);}
  }
  function json(path,limit) {
    try{return JSON.parse(read(path,limit));}
    catch(error){if(error instanceof ProjectCheckError||error?.code==='ENOENT')throw error;fail('project_json_invalid');}
  }
  let manifest;
  try {manifest=json(join(root,'action.json'),256*1024);}
  catch(error){if(error?.code==='ENOENT')fail('project_manifest_missing');throw error;}
  if(manifest?.schema_version!==1||!Array.isArray(manifest.actions)||manifest.actions.length>64)fail('project_manifest_invalid');
  const entries=[],allScriptEntries=[],allNodeEntries=[];
  for(const action of manifest.actions) {
    if(!['script','node'].includes(action?.runtime))continue;
    if(typeof action.entry!=='string'||!action.entry||isAbsolute(action.entry)||action.entry.includes('\0'))fail('project_entry_invalid');
    const entry=resolve(root,action.entry);
    const extensions=action.runtime==='node'?['.js','.mjs','.cjs']:['.js','.mjs'];
    if(!within(root,entry)||!extensions.includes(extname(entry)))fail('project_entry_invalid');
    entries.push(entry);
    (action.runtime==='node'?allNodeEntries:allScriptEntries).push(entry);
  }
  if(entries.length===0)fail('project_no_script_actions');
  let sdk;
  // 两种已支持的包布局；只在缺失时尝试根目录，不扫描或吞掉元数据/UTF8拒绝。
  for(const candidate of [join(root,'sdk/api.d.ts'),join(root,'api.d.ts')]) {
    try {read(candidate,512*1024);sdk=candidate;break;}
    catch(error){if(error?.code!=='ENOENT')throw error;}
  }
  if(!sdk)fail('project_sdk_missing');
  const rootConfig=join(root,'jsconfig.json');
  const configurations=new Map();
  // 只沿清单入口的祖先逐个尝试固定文件名，不扫描目录、不越出动作包。
  for(const entry of entries) {
    let directory=dirname(entry),configPath;
    for(;;) {
      const candidate=join(directory,'jsconfig.json');
      try {guard([candidate]);configPath=candidate;break;}
      catch(error){if(error?.code!=='ENOENT')throw error;}
      if(relative(root,directory)==='')fail('project_config_missing');
      directory=dirname(directory);
    }
    const key=process.platform==='win32'?configPath.toLowerCase():configPath;
    if(!configurations.has(key))configurations.set(key,{config:configPath,entries:[]});
    configurations.get(key).entries.push(entry);
  }
  function checkConfiguration(configPath,entries) {
    const configurationDirectory=dirname(configPath);
    const isRootConfiguration=relative(rootConfig,configPath)==='';
    const configName=relative(root,configPath).replaceAll('\\','/');
    const selected=new Set(entries);
    const scriptEntries=allScriptEntries.filter(entry=>selected.has(entry));
    const nodeEntries=allNodeEntries.filter(entry=>selected.has(entry));
    configurationReads=new Set([join(root,'action.json'),configPath]);
    sourceSnapshots.clear();
    let config;
    let configText;
    try {
      configText=read(configPath,128*1024);
      const parsed=ts.parseConfigFileTextToJson(configPath.replaceAll('\\','/'),configText);
      if(parsed.error) {
        const diagnostic=parsed.error;
        const position=diagnostic.file&&typeof diagnostic.start==='number'
          ?diagnostic.file.getLineAndCharacterOfPosition(diagnostic.start):undefined;
        fail('project_config_invalid',position?{file:configName,line:position.line+1,column:position.character+1}:undefined);
      }
      config=parsed.config;
    } catch(error) {if(error?.code==='ENOENT')fail('project_config_missing');throw error;}
    if(!config||typeof config!=='object'||Array.isArray(config)||Object.hasOwn(config,'extends'))fail('project_config_unsupported');
    const compilerOptions=Object.hasOwn(config,'compilerOptions')?config.compilerOptions:{};
    if(!compilerOptions||typeof compilerOptions!=='object'||Array.isArray(compilerOptions))fail('project_config_invalid');
    // 仅加入作者逐个列明的包内声明；不按 files/include 扫描目录或扩展 Node 入口。
    const configuredFiles=Object.hasOwn(config,'files')?config.files:[];
    if(!Array.isArray(configuredFiles)||configuredFiles.length>256
      ||configuredFiles.some(name=>typeof name!=='string'||!name||name.length>2048||name.includes('\0'))) {
      fail('project_config_invalid',{file:configName});
    }
    const configuredDeclarations=[];
    for(const name of configuredFiles) {
      if(!name.endsWith('.d.ts'))continue;
      const declaration=resolve(configurationDirectory,name);
      if(isAbsolute(name)||/[\0*?:]/.test(name)||!within(root,declaration)) {
        fail('project_config_unsupported',{file:configName});
      }
      configuredDeclarations.push(declaration);
    }
    const converted=ts.convertCompilerOptionsFromJson(compilerOptions,configurationDirectory);
    if(converted.errors.length) {
      // 转换API不带源位置；用同一配置文本取得TS已有的位置，不读取extends/include或文件。
      const configSource=ts.parseJsonText(configName,configText);
      const expression=configSource.statements[0]?.expression;
      const property=expression&&ts.isObjectLiteralExpression(expression)
        ? [...expression.properties].reverse().find(node=>ts.isPropertyAssignment(node)&&node.name.text==='compilerOptions')
        : undefined;
      const block=property?.initializer;
      const located=ts.parseJsonSourceFileConfigFileContent(configSource,{
        useCaseSensitiveFileNames:true,readDirectory:()=>[],fileExists:()=>false,readFile:()=>undefined,
      },configurationDirectory).errors.find(error=>error.file===configSource&&typeof error.start==='number'
        &&block&&error.start>=block.getStart(configSource)&&error.start<block.end
        &&converted.errors.some(original=>original.code===error.code));
      const position=located?configSource.getLineAndCharacterOfPosition(located.start):undefined;
      fail('project_config_invalid',{file:configName,
        ...(position?{line:position.line+1,column:position.character+1}:{})});
    }
    // 混合包保持Script的无隐式环境约定；不把Node全局类型注入同步入口。
    if(nodeEntries.length&&scriptEntries.length&&converted.options.types?.length) {
      fail('project_mixed_types_unsupported',{file:configName});
    }
    const options={strict:true,target:ts.ScriptTarget.ES2020,module:ts.ModuleKind.ESNext,
      moduleResolution:ts.ModuleResolutionKind.Bundler,lib:['lib.es2020.d.ts'],...converted.options,
      // 旧 .js 入口实际各自在函数体内执行；noEmit 检查也按文件隔离，不合并作者局部声明。
      noEmit:true,allowJs:true,checkJs:true,
      types:nodeEntries.length&&!scriptEntries.length?(converted.options.types??[]):[],moduleDetection:ts.ModuleDetectionKind.Force};
    const host=ts.createCompilerHost(options);
    let fatal;
    const originalExists=host.fileExists;
    const allowed=path=>within(root,resolve(path))||within(library,resolve(path));
    host.getCurrentDirectory=()=>configurationDirectory;
    host.fileExists=path=>allowed(path)&&originalExists(path);
    host.realpath=path=>{if(allowed(path))guard([path]);return resolve(path);};
    // 不做环境目录发现；只解析清单入口、显式引用和已定位的标准库。
    host.getDirectories=()=>[];
    host.readDirectory=()=>[];
    host.directoryExists=path=>{
      if(!allowed(path))return false;
      try {guard([path]);return lstatSync(path).isDirectory();}catch(error){if(error?.code==='ENOENT')return false;throw error;}
    };
    host.readFile=path=>{
      if(!allowed(path))return undefined;
      try{
        const text=read(path,within(library,resolve(path))?2*1024*1024:path.endsWith('.d.ts')?512*1024:128*1024);
        // 仅已读Node声明入口的显式path引用；不枚举依赖目录或预读声明内容。
        const name=relative(root,resolve(path)).replaceAll('\\','/');
        if(text!==undefined&&!options.noResolve&&within(root,resolve(path))
            &&/(?:^|\/)node_modules\/@types\/node\/index\.d\.ts$/i.test(name)) {
          const references=ts.preProcessFile(text,false).referencedFiles;
          if(references.length>0&&references.length<=128) {
            const directory=dirname(resolve(path));
            const declarations=references.map(item=>resolve(directory,item.fileName));
            if(references.every((item,index)=>!isAbsolute(item.fileName)&&/^[a-z0-9_./-]+\.d\.ts$/i.test(item.fileName)
                &&item.fileName.split('/').every(part=>part!=='.'&&part!=='..'&&part!=='')
                &&within(directory,declarations[index]))) {
              try {guard(declarations);}
              catch(error) {if(error?.code!=='ENOENT')throw error;} // 缺失仍交回TS原诊断/逐项守卫。
            }
          }
        }
        return text;
      }
      catch(error){
        if(error?.code==='ENOENT')return undefined;
        // TypeScript可将readFile异常转成普通诊断；单独保留本工具的范围/容量拒绝。
        fatal??=error instanceof ProjectCheckError?error:new ProjectCheckError('project_read_failed');
        return undefined;
      }
    };
    host.writeFile=()=>fail('project_emit_forbidden');
    const program=ts.createProgram([...new Set([...entries,...(isRootConfiguration?[sdk]:[]),...configuredDeclarations])],options,host);
    if(fatal)throw fatal;
    const diagnostics=ts.getPreEmitDiagnostics(program).map(item=>{
      const location=item.file&&typeof item.start==='number'?item.file.getLineAndCharacterOfPosition(item.start):undefined;
      return {code:item.code,message:ts.flattenDiagnosticMessageText(item.messageText,'\n'),
        ...(item.file?{file:relative(root,resolve(item.file.fileName)).replaceAll('\\','/')}:{ }),
        ...(location?{line:location.line+1,column:location.character+1}:{})};
    });
    if(fatal)throw fatal;
    const localName=path=>relative(root,path).replaceAll('\\','/');
    const moduleChecks=[],entryChecks=[],skippedEntries=[];
    for(const entry of new Set(scriptEntries)) {
      const name=localName(entry),legacy=extname(entry)!=='.mjs';
      const entryResult=analyzeScriptEntry({file:name,text:sourceSnapshots.get(entry)});
      let moduleResult;
      const attempted=new Set();
      for(;;) {
        const sources=[...sourceSnapshots].map(([path,text])=>({path:localName(path),text}));
        moduleResult=analyzeScriptModules({entry:name,sources,...(legacy?{protocol:'legacy_function_body'}:{})});
        // TypeScript可能把 ./x.js 解析到 x.ts；轻量宿主需要的是明确的 x.js。
        // 只补读分析器给出的包内静态边，不枚举目录、不追踪动态表达式。
        const missing=[...new Set(moduleResult.edges.filter(edge=>edge.kind==='static')
          .map(edge=>resolve(root,edge.to)).filter(path=>!sourceSnapshots.has(path)&&!attempted.has(path)))];
        if(missing.length===0||moduleResult.diagnostics.some(d=>['module_total_limit','snapshot_limit'].includes(d.code)))break;
        for(const path of missing) {
          attempted.add(path);
          if(!within(root,path))fail('project_entry_invalid');
          try {read(path,128*1024);}
          catch(error) {if(error?.code!=='ENOENT')throw error;}
        }
      }
      if(legacy&&entryResult.dependencyAnalysisComplete===false)moduleResult={...moduleResult,staticGraphComplete:false};
      moduleChecks.push({entry:name,...(legacy?{entryProtocol:'legacy_function_body'}:{}),...moduleResult});
      entryChecks.push({entry:name,...entryResult});
    }
    const nodeEntryChecks=[...new Set(nodeEntries)].map(entry=>({entry:localName(entry),
      ...analyzeNodeEntries({projectDirectory:root,typescript:ts,program,entries:[entry]})}));
    if(fatal)throw fatal;
    const environmentDiagnostics=[
      ...moduleChecks.flatMap(check=>check.diagnostics.map(item=>({entry:check.entry,category:'modules',...item}))),
      ...entryChecks.flatMap(check=>check.diagnostics.map(item=>({entry:check.entry,category:'entry',...item}))),
      ...nodeEntryChecks.flatMap(check=>check.diagnostics.map(item=>({entry:check.entry,category:'entry',runtime:'node',...item}))),
    ];
    const staticEnvironmentPassed=moduleChecks.every(check=>check.passed)
      &&entryChecks.every(check=>check.passed)&&nodeEntryChecks.every(check=>check.passed);
    return {passed:diagnostics.length===0&&staticEnvironmentPassed,compilerVersion:ts.version,project:root,
      entries:entries.map(path=>relative(root,path).replaceAll('\\','/')),diagnostics,
      checkedFiles:[...(isRootConfiguration&&configurations.size===1?loaded.keys():configurationReads)].map(path=>relative(root,path).replaceAll('\\','/')),
      staticEnvironmentPassed,environmentDiagnostics,moduleChecks,entryChecks,skippedEntries,
      ...(nodeEntries.length?{nodeEntryChecks}:{}),
      staticAnalysisComplete:skippedEntries.length===0&&moduleChecks.every(check=>check.staticGraphComplete)
        &&entryChecks.every(check=>['local_callable_declaration','legacy_callable_declaration'].includes(check.mainExport)&&check.staticAnalysisComplete!==false)
        &&nodeEntryChecks.every(check=>check.passed&&!check.diagnostics.some(item=>item.severity==='warning')),
      runtimeCapabilitiesChecked:false,runtimeVerified:false,
      runtimeExecuted:false,filesModified:false};
  }
  const results=[...configurations.values()].map(({config,entries})=>({config,result:checkConfiguration(config,entries)}));
  configurationReads=undefined;
  if(results.length===1&&relative(rootConfig,results[0].config)==='')return results[0].result;
  const names=path=>relative(root,path).replaceAll('\\','/');
  const flatten=field=>results.flatMap(({result})=>result[field]??[]);
  const projectResults=results.map(({config,result})=>({config:names(config),entries:result.entries,
    passed:result.passed,checkedFiles:result.checkedFiles}));
  return {passed:results.every(({result})=>result.passed),compilerVersion:ts.version,project:root,
    entries:entries.map(names),diagnostics:results.flatMap(({config,result})=>
      result.diagnostics.map(item=>({...item,config:names(config)}))),
    checkedFiles:[...loaded.keys()].map(names),
    staticEnvironmentPassed:results.every(({result})=>result.staticEnvironmentPassed),
    environmentDiagnostics:results.flatMap(({config,result})=>
      result.environmentDiagnostics.map(item=>({...item,config:names(config)}))),
    moduleChecks:flatten('moduleChecks'),entryChecks:flatten('entryChecks'),skippedEntries:flatten('skippedEntries'),
    ...(allNodeEntries.length?{nodeEntryChecks:flatten('nodeEntryChecks')}:{}),
    staticAnalysisComplete:results.every(({result})=>result.staticAnalysisComplete),projects:projectResults,
    runtimeCapabilitiesChecked:false,runtimeVerified:false,runtimeExecuted:false,filesModified:false};
}

if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  try {
    if(process.argv.length!==3)fail('project_directory_required');
    const result=checkProjectTypes(process.argv[2]);console.log(JSON.stringify(result));
    if(!result.passed)process.exitCode=1;
  } catch(error) {
    console.error(JSON.stringify({passed:false,error:{code:error instanceof ProjectCheckError?error.code:'project_check_failed',
      message:'脚本工程检查未完成；请检查工程路径、SDK、配置和错误代码。',
      ...(error instanceof ProjectCheckError&&error.location?{location:error.location}:{})},runtimeExecuted:false,filesModified:false}));
    process.exitCode=1;
  }
}
