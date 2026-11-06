// 真实工程创建/刷新、类型边界及两运行时端到端验收。只运行合成文本，不调用Native业务。
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {cpSync,existsSync,lstatSync,mkdirSync,readdirSync,readFileSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';

const defaultRepo=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
function fail(code,detail){throw Object.assign(new Error(code),{code,detail});}
function parse(values){
  const allowed=['--app','--node','--host','--typescript','--case-root','--source-root','--project-tool','--repo','--npm'];const result={};
  for(let index=0;index<values.length;index+=2){const flag=values[index]==='--light-host'?'--host':values[index];if(!allowed.includes(flag)||flag in result||!values[index+1]||values[index+1].startsWith('--'))fail('invalid_arguments',flag);result[flag]=path.resolve(values[index+1]);}
  for(const flag of ['--app','--node','--host','--typescript','--case-root'])if(!result[flag])fail('required_argument_missing',flag);
  result['--repo']??=defaultRepo;result['--source-root']??=result['--repo'];result['--project-tool']??=path.join(result['--repo'],'tools/script-project.mjs');return result;
}
function guard(paths){
  const entries=new Set();
  for(const target of paths)for(let current=path.resolve(target);;current=path.dirname(current)){
    if(/^(onedrive|icloud)/i.test(path.basename(current))||/^dropbox$/i.test(path.basename(current)))fail('unsafe_cloud_path');
    if(existsSync(current)){if(lstatSync(current).isSymbolicLink())fail('unsafe_link_path');entries.add(current);}
    if(path.dirname(current)===current)break;
  }
  if(process.platform==='win32'){
    const script="$ErrorActionPreference='Stop';foreach($p in (ConvertFrom-Json $env:SDK_E2E_PATHS)){$item=Get-Item -LiteralPath $p -Force;if(([long]$item.Attributes -band 0x441400) -ne 0){exit 7}}";
    const list=[...entries];for(let index=0;index<list.length;index+=64){
      const result=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from(script,'utf16le').toString('base64')],{windowsHide:true,encoding:'utf8',timeout:15000,env:{...process.env,SDK_E2E_PATHS:JSON.stringify(list.slice(index,index+64))}});
      if(result.error||result.status!==0)fail('metadata_check_failed');
    }
  }
}
function required(file,label){if(!existsSync(file))fail('required_tool_missing',label);guard([file]);if(!lstatSync(file).isFile())fail('required_tool_invalid',label);}
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const fileHash=file=>hash(readFileSync(file));
function write(file,value){mkdirSync(path.dirname(file),{recursive:true});writeFileSync(file,typeof value==='string'?value:JSON.stringify(value,null,2)+'\n');}
function snapshot(root,omitSdk=false){
  const files={};const pending=[root];
  while(pending.length){const directory=pending.pop();guard([directory]);const children=readdirSync(directory).map(name=>path.join(directory,name));guard(children);for(const file of children){
    const relative=path.relative(root,file).replaceAll('\\','/');if(relative==='node_modules'||relative.startsWith('node_modules/'))continue;if(omitSdk&&(relative==='sdk'||relative.startsWith('sdk/')))continue;
    if(lstatSync(file).isDirectory())pending.push(file);else files[relative]=fileHash(file);
  }}return Object.fromEntries(Object.entries(files).sort(([a],[b])=>a.localeCompare(b)));
}
function progress(phase){console.log(JSON.stringify({phase,status:'passed'}));}

async function main(){
  if(process.platform!=='win32')fail('windows_required');
  const options=parse(process.argv.slice(2));
  for(const [flag,label]of [['--app','app'],['--node','node'],['--host','light-host'],['--typescript','typescript'],['--project-tool','project-tool']])required(options[flag],label);
  progress('required-tools-present');
  options['--npm']??=path.join(path.dirname(options['--node']),'node_modules/npm/bin/npm-cli.js');required(options['--npm'],'npm CLI');
  const compilerDir=path.dirname(options['--typescript']);required(path.join(compilerDir,'typescript.js'),'typescript API');required(path.join(compilerDir,'lib.es2022.d.ts'),'typescript standard library');
  guard(readdirSync(compilerDir).filter(name=>/^lib\..*\.d\.ts$/.test(name)).map(name=>path.join(compilerDir,name)));
  const sdkRoot=path.join(options['--source-root'],'crates/omnitrigger-actions/runtime');
  for(const name of ['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts'])required(path.join(sdkRoot,name),'SDK '+name);
  const root=options['--case-root'],temporary=path.join(options['--repo'],'.tmp');
  const relative=path.relative(temporary,root);if(!relative||path.isAbsolute(relative)||relative==='..'||relative.startsWith('..'+path.sep))fail('case_root_must_be_repo_tmp');
  guard([root]);if(existsSync(root))fail('case_root_exists');mkdirSync(root,{recursive:true});
  const tools=path.join(root,'tools');mkdirSync(tools);const app=path.join(tools,'application.exe'),light=path.join(tools,'light-host.exe');
  cpSync(options['--app'],app,{errorOnExist:true});cpSync(options['--host'],light,{errorOnExist:true});
  const temp=path.join(root,'tmp');mkdirSync(temp);
  const config=path.join(root,'bindings.json');write(config,'[]');
  const env=Object.fromEntries(Object.entries(process.env).filter(([key])=>
    !key.toUpperCase().startsWith('OMNITRIGGER_')
    && !['NODE_OPTIONS','NODE_PATH','BETTERCLIP_DATA_ROOT','BETTERCLIP_ISOLATED_TEST_ROOT',
      'INTELLITOOLS_DEV_LOG','INTELLITOOLS_DEV_CONSOLE'].includes(key.toUpperCase())));
  const legacyRoot=path.join(root,'legacy-data');mkdirSync(legacyRoot);
  Object.assign(env,{OMNITRIGGER_NODE:options['--node'],OMNITRIGGER_SCRIPT_HOST:light,
    BETTERCLIP_DATA_ROOT:legacyRoot,TEMP:temp,TMP:temp});
  write(path.join(root,'plugins.json'),{version:1,raishot:{enabled:false,cliPath:''},legacy:{enabled:false}});
  const operations={status:'running',zero_bindings:true,created:[],refreshed:[],rejected:[]};
  const runtimeResults={status:'running',node:null,script:null,native_business_calls:0,clipboard_calls:0,gui:0};
  const typeResults={status:'running',positive:{},stale:{},negative:{},global_node_modules_used:false};
  write(path.join(root,'tools.json'),{app:{path:app,sha256:fileHash(app)},node:{path:options['--node'],sha256:fileHash(options['--node'])},light:{path:light,sha256:fileHash(light)},typescript:{path:options['--typescript'],sha256:fileHash(options['--typescript'])},source_root:options['--source-root'],project_tool:options['--project-tool']});
  function run(program,args,expected=0,cwd=options['--repo']){const result=spawnSync(program,args,{cwd,encoding:'utf8',windowsHide:true,timeout:60000,maxBuffer:4*1024*1024,env});assert.ifError(result.error);assert.equal(result.status,expected,result.stderr+result.stdout);return{exit_code:result.status,stdout:result.stdout,stderr:result.stderr};}
  function cli(args,expected=0){const result=run(app,[...args,'--config',config],expected);return JSON.parse(expected===0?result.stdout:result.stderr||result.stdout);}
  function tool(args,expected=0){const result=run(options['--node'],[options['--project-tool'],...args],expected);return JSON.parse(expected===0?result.stdout:result.stderr);}
  function tsc(project,expected=0){return run(options['--node'],[options['--typescript'],'--project',project,'--noEmit','--skipLibCheck','false','--pretty','false'],expected);}
  const npmUser=path.join(root,'npm-user.npmrc'),npmGlobal=path.join(root,'npm-global.npmrc');write(npmUser,'');write(npmGlobal,'');
  function npmProject(project,args,expected=0){return run(options['--node'],[options['--npm'],...args,'--no-audit','--no-fund','--registry=https://registry.npmjs.org','--cache',path.join(root,'npm-cache'),'--userconfig',npmUser,'--globalconfig',npmGlobal],expected,project);}
  function installNodeTypes(project){return npmProject(project,['install','--ignore-scripts']);}
  const description=JSON.parse(run(light,['--describe-runtime']).stdout);assert.equal(description.product,'omnitrigger-script-host');assert.equal(description.api_version,1);write(path.join(root,'light-description.json'),description);
  const initialCatalog=cli(['--list-package-actions']);assert.equal(initialCatalog.actions.length,0);operations.initial_catalog_empty=true;
  progress('pinned-tools-and-light-description');
  const projects={};
  for(const [runtime,id]of [['node','node-text'],['script','sync-text']]){
    const created=tool(['create','--runtime',runtime,'--binary',app,'--config',config,'--id',id,'--source-root',options['--source-root']]);assert.equal(created.status,'created');
    projects[runtime]=created.package_root;operations.created.push(created);
    if(runtime==='node'){
      typeResults.node_missing_dependencies=npmProject(created.package_root,['run','check'],1);
      assert((typeResults.node_missing_dependencies.stdout+typeResults.node_missing_dependencies.stderr).includes('开发依赖尚未安装完整，请先运行 npm install --ignore-scripts'));
      typeResults.node_install=installNodeTypes(created.package_root);
      write(path.join(created.package_root,'node-api-check.mjs'),`import {writeFileSync} from 'node:fs';
/** @param {import('./sdk/api').ActionContext} ctx */
export function checkNodeTypes(ctx){
  writeFileSync('unused-synthetic.txt','synthetic');
  const environment=process.env.NODE_ENV;
  const response=fetch('https://example.invalid',{signal:ctx.signal});
  return {environment,response};
}
`);
    }
    if(runtime==='node')typeResults.node_check_command=npmProject(created.package_root,['run','check']);
    typeResults.positive[runtime]=tsc(path.join(created.package_root,'jsconfig.json'));
    write(path.join(root,'operations-results.json'),operations);write(path.join(root,'type-results.json'),typeResults);
  }
  progress('create-both-runtimes');
  for(const [runtime,project]of Object.entries(projects)){
    write(path.join(project,'main.mjs'),readFileSync(path.join(project,'main.mjs'),'utf8')+'\n// 作者入口修改应被保留\n');
    write(path.join(project,'lib/text.js'),`/** @param {unknown} value */\nexport function cleanText(value) { return String(value ?? '').trim() + '|module'; }\n`);
    write(path.join(project,'lib/author.js'),'export const authorValue = "保留";\n');
    write(path.join(project,'README.md'),readFileSync(path.join(project,'README.md'),'utf8')+'\n作者说明应被保留。\n');
    const call=JSON.parse(readFileSync(path.join(project,'call.json'),'utf8'));call.input={text:'  合成-'+runtime+'  '};write(path.join(project,'call.json'),call);
    const before=snapshot(project,true);
    write(path.join(project,'sdk/api.d.ts'),'// intentionally stale SDK\n');
    if(runtime==='node')write(path.join(project,'sdk/clipboard-history-api.d.ts'),'export {};\n');
    typeResults.stale[runtime]=tsc(path.join(project,'jsconfig.json'),2);
    const refreshed=tool(['refresh','--runtime',runtime,'--project',project,'--source-root',options['--source-root']]);assert.equal(refreshed.status,'refreshed');assert.deepEqual(snapshot(project,true),before);
    assert.equal(fileHash(path.join(project,'sdk/api.d.ts')),fileHash(path.join(sdkRoot,runtime==='node'?'api.d.ts':'script-types.d.ts')));
    if(runtime==='node')assert.equal(fileHash(path.join(project,'sdk/clipboard-history-api.d.ts')),fileHash(path.join(sdkRoot,'clipboard-history-api.d.ts')));
    operations.refreshed.push({runtime,author_files_preserved:true,before,reply:refreshed});
    typeResults.positive[runtime]=tsc(path.join(project,'jsconfig.json'));
    write(path.join(root,'operations-results.json'),operations);write(path.join(root,'type-results.json'),typeResults);
  }
  progress('refresh-preserves-author-files');
  for(const [runtime,project]of Object.entries(projects)){
    const reply=cli(['--run-action',path.join(project,'call.json')]);assert.equal(reply.status,'succeeded');assert.equal(reply.output.text,'合成-'+runtime+'|module');
    runtimeResults[runtime]={actual_cli_reply:reply,relative_module_result_verified:true,entry_contract:runtime==='node'?'default async main':'named synchronous main',native_business_calls:0};
    write(path.join(root,runtime+'-runtime-result.json'),runtimeResults[runtime]);
    write(path.join(root,'runtime-results.json'),runtimeResults);
  }
  progress('actual-node-and-script-runtime');
  const nodeProject=projects.node,before=snapshot(nodeProject);
  const wrong=tool(['refresh','--runtime','script','--project',nodeProject,'--source-root',options['--source-root']],1);assert.equal(wrong.error.code,'project_runtime_mismatch');assert.deepEqual(snapshot(nodeProject),before);operations.rejected.push({case:'wrong runtime refresh',reply:wrong,preserved:true});
  const exists=tool(['create','--runtime','node','--binary',app,'--config',config,'--id','node-text','--source-root',options['--source-root']],1);assert(['project_cli_rejected','project_exists'].includes(exists.error.code));assert.deepEqual(snapshot(nodeProject),before);
  assert.equal(cli(['--create-script-action','node-text'],1).error.code,'project_exists');operations.rejected.push({case:'existing project',reply:exists,preserved:true});
  const mismatch=path.join(root,'mismatched-source/crates/omnitrigger-actions/runtime');mkdirSync(mismatch,{recursive:true});
  for(const name of ['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts'])write(path.join(mismatch,name),readFileSync(path.join(sdkRoot,name),'utf8')+(name==='script-types.d.ts'?'\n// intentional SDK identity mismatch\n':''));
  const incompatible=tool(['create','--runtime','node','--binary',app,'--config',config,'--id','sdk-mismatch','--source-root',path.join(root,'mismatched-source')],1);assert.equal(incompatible.error.code,'project_sdk_binary_mismatch');
  const partial=path.join(root,'actions/sdk-mismatch');if(existsSync(partial)){const manifest=JSON.parse(readFileSync(path.join(partial,'action.json'),'utf8'));assert.equal(manifest.actions[0].runtime,'script');assert.equal(fileHash(path.join(partial,'sdk/api.d.ts')),fileHash(path.join(sdkRoot,'script-types.d.ts')));}
  assert.deepEqual(snapshot(nodeProject),before);operations.rejected.push({case:'SDK versus binary mismatch',reply:incompatible,existing_project_preserved:true,new_project_not_promoted_to_node:true});progress('required-refusals');
  const {default:ts}=await import(pathToFileURL(path.join(compilerDir,'typescript.js')).href);
  for(const [runtime,body,count]of [['node','document.title;\nwindow.location.href;\n',2],['script',"import {writeFileSync} from 'node:fs';\nprocess.cwd();\nBuffer.from('x');\nrequire('node:fs');\nglobal.process;\n__dirname;\ntype P = NodeJS.Process;\n",7]]){
    const folder=path.join(root,'negative',runtime);mkdirSync(folder,{recursive:true});const sdk=path.relative(folder,path.join(projects[runtime],'sdk/api.d.ts')).replaceAll('\\','/');
    write(path.join(folder,'probe.ts'),(runtime==='node'?`import type {ActionContext} from '${sdk}';\ndeclare const ctx:ActionContext;\n`:'')+body);
    const configuration={compilerOptions:{strict:true,target:'ES2022',module:'ESNext',moduleResolution:'Bundler',types:runtime==='node'?['node']:[],...(runtime==='node'?{typeRoots:[path.join(projects.node,'node_modules/@types')]}:{}),lib:['ES2022'],noEmit:true,skipLibCheck:false},files:[sdk,'probe.ts'],include:[]};write(path.join(folder,'tsconfig.json'),configuration);
    const parsed=ts.parseJsonConfigFileContent(configuration,ts.sys,folder),program=ts.createProgram(parsed.fileNames,parsed.options),diagnostics=ts.getPreEmitDiagnostics(program).filter(d=>d.category===ts.DiagnosticCategory.Error);
    assert.equal(diagnostics.length,count,ts.formatDiagnostics(diagnostics,{getCurrentDirectory:()=>folder,getCanonicalFileName:file=>file,getNewLine:()=> '\n'}));
    const loaded=program.getSourceFiles().map(file=>path.resolve(file.fileName));assert(!loaded.some(file=>/^lib\.dom(?:\.|$)/i.test(path.basename(file))));
    if(runtime==='script')assert(!loaded.some(file=>/[\\/]@types[\\/]node[\\/]/i.test(file)));
    if(runtime==='node')assert(loaded.some(file=>/[\\/]@types[\\/]node[\\/]/i.test(file)));
    typeResults.negative[runtime]={actual_tsc:tsc(path.join(folder,'tsconfig.json'),2),diagnostics:diagnostics.map(d=>({code:d.code,message:ts.flattenDiagnosticMessageText(d.messageText,' ')})),dom_library_loaded:false,node_environment_types_loaded:runtime==='node'};
  }
  assert.equal(readFileSync(config,'utf8'),'[]');
  operations.status='passed';runtimeResults.status='passed';typeResults.status='passed';typeResults.compiler_version=ts.version;
  write(path.join(root,'operations-results.json'),operations);write(path.join(root,'runtime-results.json'),runtimeResults);write(path.join(root,'type-results.json'),typeResults);
  const summary={status:'passed',root,created_runtimes:2,refreshed_runtimes:2,actual_runtime_passes:2,rejection_cases:3,node_browser_global_diagnostics:2,script_node_global_diagnostics:6,script_node_module_diagnostics:1,zero_bindings:true,native_business_calls:0,clipboard_calls:0,gui:0};write(path.join(root,'result.json'),summary);console.log(JSON.stringify(summary));
}
main().catch(error=>{console.error(JSON.stringify({status:'failed',error:{code:error.code??'assertion_failed',detail:error.detail??error.message},skipped:false}));process.exitCode=1;});
