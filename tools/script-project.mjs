// 开发工程维护：复用应用的创建CLI，只分发声明与Node入口，不加载或运行作者代码。
import {spawnSync} from 'node:child_process';
import {randomUUID,createHash} from 'node:crypto';
import {existsSync,lstatSync,mkdirSync,readFileSync,renameSync,unlinkSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const sourceDefault=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
// 每一批只核验调用处明确列出的路径；不跨批缓存属性，写入仍逐文件重新guard。
function localPath(value,missing=false,readOnly=false,parts=new Set()){
  const full=path.resolve(value);
  for(let p=full;;p=path.dirname(p)){
    if(/^(onedrive|icloud)/i.test(path.basename(p))||/^dropbox$/i.test(path.basename(p)))throw Error('project_cloud_path');
    if(existsSync(p)){
      const info=lstatSync(p);if(info.isSymbolicLink()||!readOnly&&info.isFile()&&info.nlink>1)throw Error('project_link_path');parts.add(p);
    }else if(!missing)throw Error('project_path_missing');
    if(path.dirname(p)===p)break;
  }
  return full;
}
function guardMany(requests){
  const parts=new Set();
  const full=requests.map(({value,missing=false,readOnly=false})=>localPath(value,missing,readOnly,parts));
  if(process.platform==='win32'){
    const script="$ErrorActionPreference='Stop';foreach($p in (ConvertFrom-Json $env:OMNI_PROJECT_PATHS)){$item=Get-Item -LiteralPath $p -Force;if(([long]$item.Attributes -band 0x441400) -ne 0){exit 7}}";
    const result=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from(script,'utf16le').toString('base64')],
      {windowsHide:true,encoding:'utf8',timeout:15000,env:{...process.env,OMNI_PROJECT_PATHS:JSON.stringify([...parts])}});
    if(result.error||result.status!==0)throw Error('project_metadata_unavailable');
  }
  return full;
}
function guard(value,missing=false,readOnly=false){return guardMany([{value,missing,readOnly}])[0];}
function readMany(files,roots=[]){
  guardMany([...roots.map(value=>({value})),...files.map(value=>({value}))]);
  return files.map(file=>{
    // 批内实际读取前仍重新检查名称、链接与hardlink，不复用旧lstat结果。
    localPath(file);
    if(!lstatSync(file).isFile()||lstatSync(file).size>512*1024)throw Error('project_file_invalid');
    return readFileSync(file,'utf8');
  });
}
function read(file){return readMany([file])[0];}
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
// 已确认的公共canonical声明，保留各版本LF/CRLF字节指纹；未知版本不猜测。
const canonicalSdkHashes={
  "api.d.ts": [
    "495ab8e791cf0a097f1d4a9981ba9c8d42b6fc474b2e46fc3b22ed8954e893ea",
    "1529bbe80cbfaad91e0a8b1843b83fc7e2a5b46c761333f3a62b1f417a787c40",
    "12020f626e467a1025b8eb9b08245952f0425f97a983a8f66c48a1864e6d7adc",
    "aacd818ae060a0c4391d322fb0d42b6712b01a582fef9ff3821bc150911a9a14",
    "ec277716a63ae07c55c36ffa58b58aaf92998b6219ca722539cc2a01373bf2a4",
    "06eb52f97df7a63cf430c813b5e9ca05e2370b6d54fc77d40379c2ef0c2faf58",
    "e6338de5fce25b6864c59d1344785e10240f092b351b487ac82be336cf89849d",
    "c33bc088d6c8bb0d48fb0c97b5f1d0977018fb58ba0cc6f49d0d435fb68e6f4c",
    "d893ac66aba736a6093ff15a9498e010d5df35bd2ec0c88fceae2ae1ab1f8798",
    "54e9c5c8bc7abf1437f52c02cd4536523f7210f3478c6b56a519edb6d311f771",
    "937f423a88e6565bc2130bcb4b9be28d9907d43511118fcfb9795d6b723c4284",
    "521db15d9df3c83f880a38b58dd22155b8a02e80d51f5daefda3fa4763e2c85d",
    "69f67384a1768a7b7987c3b595328184773b1d624e6996fd0574350dcdbf680b",
    "1087192989a1692d64e3f53efbaf4be46739c39852490efd2146104c591923fa",
    "7ae85df4d6bba81754d694e4df97ac27edc9c8bab1ddb646c9f4c396c401b72d",
    "67aa215ce5340b2fcddfa460eaef89d9e9b66acac0511acfc61bdc4593220733",
    "a2da2d9f3d4da9a1d50b2bb94671c157cddc3176d8d67e204486a907e3afb391",
    "4e536fee2b28f8408e9183cbddf2017daffec99d175607e7d34dd35eb81462fd",
    "2aa8641ecdcc6e72d39bfbed8fd8bf3457458be787d73f9c510194eea3a78445",
    "deb1ba71d70b183680f0a94e4a87ff0f51697c05fcbedec6e4d719acd81e5d7c",
    "a9e655eb47154a49a38f63c9e4c320b00a1d393d60c7c065fbb542c455c932d5",
    "164874388d6b6394a1a62e361c2d9a898f021483727b32479a1a79c415cbcccd"
  ],
  "script-types.d.ts": [
    "230fa9343b86cbe359d436c75f9abdd36f7c5e4f17742856944a6371e10e5e1d",
    "d0c651d9deefaf9523449446e899057d95948bb9c743a1db3cab0fd8d2daf6f5",
    "261666bf8be38669cbbe16f27fa2bf44f3a20f2ded90cef2be888de85a939f01",
    "16e2652d5f012d0d0ddcf9104462179dcaa3366b3d3a32d0e349d5fdd096b427",
    "1e53113a470d14b5be14aef9f75c1365bbf447c20effcf2f160108aa18c2d5df",
    "561163bdf3db853faa7ef2a3ad4717707a1b47906015df0f28538aa6d9349b75",
    "e65e3fb619082bd1e17edcdd2a7e6f5ad8ddc14b1c268f3874098344266daa1e",
    "a3ef947e96503643a863dc4224f50fd39317da5c4c60264041bb238f07247f77",
    "25d557d344f69c3e7ce0184b79705fc0e61471eb9e365bd39bb779d04e40a248",
    "b524985ebfbe4c9a92415bf34f85ac354a34fc97a4b51c2b25436fda673f5711",
    "32c85e7bb9fd47acb89991fcabf72a1f30da6d961e456c33d35af508066a9721",
    "ed5651429e554302dd41ccbf28998b736f7ee99f1df0c80cabbe0ad3c3f696fb",
    "a483323404fdb821e08e2426f4dad17dda9ae697a561d1610794506a999b237b",
    "bdc60286993cbd047821c17336c9c13fcf5f6b5e84b5fa2d774bfa1f1f6e9350",
    "b01bb12f8f79e07f1bd51f771a150f0a23b2e4417d180eb2d100a24715827369",
    "d820919aa42c3b8c7388e21da3287e74cac6b67f1be348d2159948ceeb814527",
    "e37d19588d47d2232593573f71f2391179712ef95f8d641e3defc9157c09fff8",
    "018f73eb09492cda3defa4242f6507bbf4592b986dfe11e122be7957a33c3ab0",
    "0ed007e227c86b53a94d03d9aa6c6ffba029b789fd7321a089dbb795af556b61",
    "d1eab6559992e59cb0133d03ece731f8202cf22833ac6d86fff2f6621575333f",
    "2cee7b1c828ebc16eaecbc288a49f26e184c98f020e771c48791e8a4fec145f1",
    "b08e98fbc9b7864407f64c092b3474a454e835ba1831670337d085c91402a67b"
  ],
  "clipboard-history-api.d.ts": [
    "c40fd66fd6318909c1bab62775d097dcfb5a8a7a84a411883a4852bdbd04cf2e",
    "b8089bf8e920c3c818474f739c3f862ba32dffc4a1e3dfde64b647c023031134"
  ],
  "script-api.d.ts": [
    "c623dc7ead235801d87412dd5747f34dd0be7c410686a331e9fdbe23efd90c15",
    "cdeaa2ec34e023a58b93b1b620ad6e366db1eff33118f60f9a400fec748290de"
  ]
};
function portableSdkLayout(project,manifest){
  const candidates=['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts',
    'sdk/api.d.ts','sdk/script-types.d.ts','sdk/script-api.d.ts','sdk/clipboard-history-api.d.ts'];
  guardMany(candidates.map(name=>({value:path.join(project,name),missing:true})));
  const present=name=>existsSync(path.join(project,name));
  const flat=['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts'].some(present);
  const hasScript=manifest.actions.some(action=>action?.runtime==='script');
  let nested=['sdk/script-types.d.ts','sdk/script-api.d.ts'].some(present)
    ||hasScript&&present('sdk/clipboard-history-api.d.ts');
  // Script包中的Node声明不是旧生成布局，不能把缺失的其余声明当作legacy刷新。
  if(!nested&&hasScript&&present('sdk/api.d.ts'))nested=read(path.join(project,'sdk/api.d.ts')).includes('export interface ActionContext');
  if(flat&&(nested||present('sdk/api.d.ts')))throw Error('project_sdk_layout_ambiguous');
  if(!flat&&!nested)return null;
  const prefix=flat?'':'sdk/';
  const names=['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts'];
  if(names.some(name=>!present(prefix+name)))throw Error('project_sdk_layout_incomplete');
  if(nested&&present('sdk/script-api.d.ts'))names.push('script-api.d.ts');
  return {prefix,names};
}
function refreshPortableSdk(project,source,runtime,layout){
  const sourceRoot=path.join(source,'crates/omnitrigger-actions/runtime');
  const files=layout.names.map(name=>path.join(project,layout.prefix+name));
  const texts=readMany([...files,...layout.names.map(name=>path.join(sourceRoot,name))]);
  const data=layout.names.map((name,index)=>{
    const file=files[index],before=texts[index],text=texts[files.length+index];
    const digest=createHash('sha256').update(before,'utf8').digest('hex');
    if(before!==text&&!canonicalSdkHashes[name].includes(digest))throw Error('project_sdk_unrecognized');
    return {file,before,text};
  });
  // 所有声明先通过归属校验；写入前再复查，作者同时改动时拒绝覆盖。
  for(const item of data){
    if(read(item.file)!==item.before)throw Error('project_sdk_changed');
    if(item.before!==item.text)write(item.file,item.text);
  }
  return {status:'refreshed',runtime,package_root:project,layout:layout.prefix?'sdk':'flat',
    files:layout.names.map(name=>layout.prefix+name),author_files_changed:false};
}
function refresh(project,source,runtime){
  project=path.resolve(project);const [manifestText]=readMany([path.join(project,'action.json')],[project]);
  const manifest=JSON.parse(manifestText);
  if(manifest.schema_version!==1||!Array.isArray(manifest.actions)||!manifest.actions.length
      ||!['node','script'].includes(runtime))throw Error('project_runtime_mismatch');
  const layout=portableSdkLayout(project,manifest);
  if(layout){
    if(manifest.actions.some(action=>!['node','script'].includes(action?.runtime))
        ||!manifest.actions.some(action=>action.runtime===runtime))throw Error('project_runtime_mismatch');
    return refreshPortableSdk(project,source,runtime,layout);
  }
  if(manifest.actions.some(action=>action.runtime!==runtime))throw Error('project_runtime_mismatch');
  const root=path.join(source,'crates/omnitrigger-actions/runtime');
  const names=runtime==='node'?['api.d.ts','clipboard-history-api.d.ts']:['script-types.d.ts'];
  const files=names.map(name=>path.join(root,name));
  if(runtime==='node'){names.push('check-node-entry.cjs');files.push(path.join(source,'tools/script-sdk-check/check-node-entry.cjs'));}
  const texts=readMany(files);
  const data=names.map((name,index)=>({name,text:texts[index]}));
  const sdk=path.join(project,'sdk');guard(sdk,true);if(!existsSync(sdk))mkdirSync(sdk);
  for(const item of data)write(path.join(sdk,runtime==='script'?'api.d.ts':item.name),item.text);
  return{status:'refreshed',runtime,package_root:project,files:data.map(item=>'sdk/'+(runtime==='script'?'api.d.ts':item.name)),author_files_changed:false};
}

// create失败可能留下完整或部分工程；这些诊断不代表已回滚或未写入。
let createDiagnostic;
const boundedErrorCode=value=>typeof value==='string'&&/^[A-Z][A-Z0-9_]{0,63}$/.test(value)?value:'unknown';
try{
  const operation=process.argv[2];const args=options(process.argv.slice(3));
  const allowed=operation==='create'?['--binary','--config','--id','--runtime','--source-root']:['--project','--runtime','--source-root'];
  if(!['create','refresh'].includes(operation)||Object.keys(args).some(key=>!allowed.includes(key))||!['node','script'].includes(args['--runtime']))throw Error('invalid_arguments');
  let source=args['--source-root']??sourceDefault;const runtime=args['--runtime'];
  if(operation!=='create')source=guard(source);
  let project=args['--project'];let created;
  if(operation==='create'){
    if(!args['--binary']||!args['--config']||!args['--id'])throw Error('invalid_arguments');
    const inputs=guardMany([{value:source},{value:args['--binary'],readOnly:true},{value:args['--config']}]);
    source=inputs[0];const binary=inputs[1],config=inputs[2];
    const expectedProject=path.join(path.dirname(config),'actions',args['--id']);
    createDiagnostic={stage:'application_create',project_state:'unconfirmed',
      ...(/^[a-z0-9][a-z0-9_-]{0,63}$/i.test(args['--id'])?{project_root_to_check:expectedProject}:{}),cli_timeout_ms:60000};
    const began=performance.now();
    const result=spawnSync(binary,['--create-script-action',args['--id'],'--config',config],{encoding:'utf8',windowsHide:true,timeout:60000,maxBuffer:4*1024*1024});
    Object.assign(createDiagnostic,{cli_elapsed_ms:Math.max(0,Math.round(performance.now()-began)),
      cli_exit_code:Number.isInteger(result.status)?result.status:null,
      cli_signal:typeof result.signal==='string'&&/^SIG[A-Z0-9]{1,15}$/.test(result.signal)?result.signal:null});
    if(result.error){createDiagnostic.spawn_error_code=boundedErrorCode(result.error.code);throw Error('project_cli_unavailable');}
    if(result.status!==0)throw Error('project_cli_rejected');
    createDiagnostic.stage='application_reply';
    created=JSON.parse(result.stdout);project=created?.project?.package_root;
    if(created?.status!=='created'||project!==expectedProject)throw Error('project_cli_invalid_reply');
    createDiagnostic.stage='sdk_validation';
    const sdkPair=readMany([path.join(project,'sdk/api.d.ts'),path.join(source,'crates/omnitrigger-actions/runtime/script-types.d.ts')]);
    if(sdkPair[0].replaceAll('\r\n','\n')!==sdkPair[1].replaceAll('\r\n','\n'))throw Error('project_sdk_binary_mismatch');
    if(runtime==='node'){
      createDiagnostic.stage='node_adaptation';
      const file=path.join(project,'action.json');const manifest=JSON.parse(read(file));
      if(manifest.actions.length!==1||manifest.actions[0].runtime!=='script'||manifest.actions[0].entry!=='main.mjs')throw Error('project_cli_invalid_reply');
      // 应用已生成入口、相对模块、call、配置；只适配新Node工程的入口、模块格式与说明。
      write(path.join(project,'main.mjs'),nodeEntry);
      const packageFile=path.join(project,'package.json');guard(packageFile,true);
      writeFileSync(packageFile,JSON.stringify({private:true,type:'module',engines:{node:'>=22'},
        scripts:{
          precheck:`node --input-type=commonjs -e "if(!require('node:fs').existsSync('node_modules/typescript/bin/tsc')||!require('node:fs').existsSync('node_modules/@types/node/index.d.ts')){console.error('开发依赖尚未安装完整，请先运行 npm install --ignore-scripts，再运行 npm run check。');process.exit(1)}"`,
          check:'node ./node_modules/typescript/bin/tsc --project jsconfig.json && node ./sdk/check-node-entry.cjs .'},
        devDependencies:{'@types/node':'22.20.5',typescript:'5.9.3'}},null,2)+'\n',{flag:'wx'});
      const configFile=path.join(project,'jsconfig.json');const config=JSON.parse(read(configFile));
      Object.assign(config.compilerOptions,{target:'ES2022',lib:['ES2022'],types:['node']});
      write(configFile,JSON.stringify(config,null,2)+'\n');
      manifest.actions[0].runtime='node';write(file,JSON.stringify(manifest,null,2)+'\n');
      write(path.join(project,'README.md'),`这是 Node.js 22 或更高版本的 ESM 工程。动作入口必须 default 导出函数。API 返回 Promise，调用时使用 await；包内模块使用相对导入。

在工程目录显式安装开发依赖并检查：

    npm install --ignore-scripts
    npm run check

开发依赖固定 TypeScript 5.9.3 与 @types/node 22.20.5，支持 node:fs、process、fetch 等 Node 22 API 的严格检查；只加载 ES2022 和 Node 声明，不加入 window、document 等浏览器全局。首次安装生成 package-lock.json，后续可用 npm ci --ignore-scripts 复现依赖。 未安装完整时，npm run check 会明确提示安装命令；检查固定使用本工程的 TypeScript，不从全局 PATH 查找 tsc。 通过类型检查后还会按 action.json 检查 Node ESM 入口的 default 可调用导出；缺少 default 或明确非函数值会失败，不执行作者代码。动态类型或 CommonJS 入口会明确报告未确认，不冒充运行时通过。

创建工程只写本地文件，不下载依赖、不执行 npm 或作者脚本。类型检查不执行动作代码；实际运行仍需 Node。SDK 可用本开发工具显式 refresh 更新；刷新只替换 SDK，作者 package.json、jsconfig.json、入口及说明保持。 已有工程刷新后可显式运行 node sdk/check-node-entry.cjs .；刷新不会改写作者的 check 命令。
`);
    }
  }
  if(!project)throw Error('invalid_arguments');
  if(createDiagnostic)createDiagnostic.stage='sdk_refresh';
  const reply=refresh(project,source,runtime);
  console.log(JSON.stringify({...reply,status:operation==='create'?'created':reply.status,application_created:!!created,author_code_executed:false}));
}catch(error){const code=/^[a-z][a-z0-9_]+$/.test(error?.message??'')?error.message:'project_failed';console.error(JSON.stringify({status:'failed',error:{code,...createDiagnostic},author_code_executed:false}));process.exitCode=1;}
