// 开发工程维护：复用应用的创建CLI，只分发声明与Node入口，不加载或运行作者代码。
import {spawnSync} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {existsSync,lstatSync,mkdirSync,readFileSync,renameSync,unlinkSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const sourceDefault=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
function guard(value,missing=false,readOnly=false){
  const full=path.resolve(value);const parts=[];
  for(let p=full;;p=path.dirname(p)){
    if(/^(onedrive|icloud)/i.test(path.basename(p))||/^dropbox$/i.test(path.basename(p)))throw Error('project_cloud_path');
    if(existsSync(p)){
      const info=lstatSync(p);if(info.isSymbolicLink()||!readOnly&&info.isFile()&&info.nlink>1)throw Error('project_link_path');parts.push(p);
    }else if(!missing)throw Error('project_path_missing');
    if(path.dirname(p)===p)break;
  }
  if(process.platform==='win32'){
    const script="$ErrorActionPreference='Stop';foreach($p in (ConvertFrom-Json $env:OMNI_PROJECT_PATHS)){$item=Get-Item -LiteralPath $p -Force;if(([long]$item.Attributes -band 0x441400) -ne 0){exit 7}}";
    const result=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from(script,'utf16le').toString('base64')],
      {windowsHide:true,encoding:'utf8',timeout:15000,env:{...process.env,OMNI_PROJECT_PATHS:JSON.stringify(parts)}});
    if(result.error||result.status!==0)throw Error('project_metadata_unavailable');
  }
  return full;
}
function read(file){guard(file);if(!lstatSync(file).isFile()||lstatSync(file).size>512*1024)throw Error('project_file_invalid');return readFileSync(file,'utf8');}
function write(file,text){
  guard(file,true);const temporary=file+'.sdk-'+randomUUID()+'.tmp';
  let owned=false;
  try{writeFileSync(temporary,text,{flag:'wx'});owned=true;renameSync(temporary,file);owned=false;}
  finally{if(owned)unlinkSync(temporary);}
}
function options(values){
  const result={};for(let i=0;i<values.length;i+=2){
    if(!/^--[a-z-]+$/.test(values[i])||!values[i+1]||values[i+1].startsWith('--')||values[i] in result)throw Error('invalid_arguments');
    result[values[i]]=values[i+1];
  }return result;
}
const nodeEntry=`import { cleanText } from './lib/text.js';

/** @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx) {
    return { text: cleanText(ctx.input.text), invocation: ctx.invocation };
}
`;
function refresh(project,source,runtime){
  project=guard(project);const manifest=JSON.parse(read(path.join(project,'action.json')));
  if(manifest.schema_version!==1||!Array.isArray(manifest.actions)||!manifest.actions.length
      ||manifest.actions.some(action=>action.runtime!==runtime)||!['node','script'].includes(runtime))throw Error('project_runtime_mismatch');
  const root=path.join(source,'crates/omnitrigger-actions/runtime');
  const names=runtime==='node'?['api.d.ts','clipboard-history-api.d.ts']:['script-types.d.ts'];
  const data=names.map(name=>({name,text:read(path.join(root,name))}));
  const sdk=path.join(project,'sdk');guard(sdk,true);if(!existsSync(sdk))mkdirSync(sdk);
  for(const item of data)write(path.join(sdk,runtime==='script'?'api.d.ts':item.name),item.text);
  return{status:'refreshed',runtime,package_root:project,files:data.map(item=>'sdk/'+(runtime==='script'?'api.d.ts':item.name)),author_files_changed:false};
}

try{
  const operation=process.argv[2];const args=options(process.argv.slice(3));
  const allowed=operation==='create'?['--binary','--config','--id','--runtime','--source-root']:['--project','--runtime','--source-root'];
  if(!['create','refresh'].includes(operation)||Object.keys(args).some(key=>!allowed.includes(key))||!['node','script'].includes(args['--runtime']))throw Error('invalid_arguments');
  const source=guard(args['--source-root']??sourceDefault);const runtime=args['--runtime'];
  let project=args['--project'];let created;
  if(operation==='create'){
    if(!args['--binary']||!args['--config']||!args['--id'])throw Error('invalid_arguments');
    const binary=guard(args['--binary'],false,true);const config=guard(args['--config']);
    const result=spawnSync(binary,['--create-script-action',args['--id'],'--config',config],{encoding:'utf8',windowsHide:true,timeout:30000,maxBuffer:4*1024*1024});
    if(result.error)throw Error('project_cli_unavailable');
    if(result.status!==0)throw Error('project_cli_rejected');
    created=JSON.parse(result.stdout);project=created?.project?.package_root;
    if(created?.status!=='created'||project!==path.join(path.dirname(config),'actions',args['--id']))throw Error('project_cli_invalid_reply');
    if(read(path.join(project,'sdk/api.d.ts')).replaceAll('\r\n','\n')!==read(path.join(source,'crates/omnitrigger-actions/runtime/script-types.d.ts')).replaceAll('\r\n','\n'))throw Error('project_sdk_binary_mismatch');
    if(runtime==='node'){
      const file=path.join(project,'action.json');const manifest=JSON.parse(read(file));
      if(manifest.actions.length!==1||manifest.actions[0].runtime!=='script'||manifest.actions[0].entry!=='main.mjs')throw Error('project_cli_invalid_reply');
      // 应用已生成入口、相对模块、call、配置；只适配新Node工程的入口、模块格式与说明。
      write(path.join(project,'main.mjs'),nodeEntry);
      const packageFile=path.join(project,'package.json');guard(packageFile,true);
      writeFileSync(packageFile,JSON.stringify({private:true,type:'module',engines:{node:'>=22'},
        scripts:{
          precheck:`node --input-type=commonjs -e "if(!require('node:fs').existsSync('node_modules/typescript/bin/tsc')||!require('node:fs').existsSync('node_modules/@types/node/index.d.ts')){console.error('开发依赖尚未安装完整，请先运行 npm install --ignore-scripts，再运行 npm run check。');process.exit(1)}"`,
          check:'node ./node_modules/typescript/bin/tsc --project jsconfig.json'},
        devDependencies:{'@types/node':'22.20.5',typescript:'5.9.3'}},null,2)+'\n',{flag:'wx'});
      const configFile=path.join(project,'jsconfig.json');const config=JSON.parse(read(configFile));
      Object.assign(config.compilerOptions,{target:'ES2022',lib:['ES2022'],types:['node']});
      write(configFile,JSON.stringify(config,null,2)+'\n');
      manifest.actions[0].runtime='node';write(file,JSON.stringify(manifest,null,2)+'\n');
      write(path.join(project,'README.md'),`这是 Node.js 22 或更高版本的 ESM 工程。动作入口必须 default 导出函数。API 返回 Promise，调用时使用 await；包内模块使用相对导入。

在工程目录显式安装开发依赖并检查：

    npm install --ignore-scripts
    npm run check

开发依赖固定 TypeScript 5.9.3 与 @types/node 22.20.5，支持 node:fs、process、fetch 等 Node 22 API 的严格检查；只加载 ES2022 和 Node 声明，不加入 window、document 等浏览器全局。首次安装生成 package-lock.json，后续可用 npm ci --ignore-scripts 复现依赖。 未安装完整时，npm run check 会明确提示安装命令；检查固定使用本工程的 TypeScript，不从全局 PATH 查找 tsc。

创建工程只写本地文件，不下载依赖、不执行 npm 或作者脚本。类型检查不执行动作代码；实际运行仍需 Node。SDK 可用本开发工具显式 refresh 更新；刷新只替换 SDK，作者 package.json、jsconfig.json、入口及说明保持。
`);
    }
  }
  if(!project)throw Error('invalid_arguments');
  const reply=refresh(project,source,runtime);
  console.log(JSON.stringify({...reply,status:operation==='create'?'created':reply.status,application_created:!!created,author_code_executed:false}));
}catch(error){const code=/^[a-z][a-z0-9_]+$/.test(error?.message??'')?error.message:'project_failed';console.error(JSON.stringify({status:'failed',error:{code},author_code_executed:false}));process.exitCode=1;}
