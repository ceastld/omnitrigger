import ts from 'typescript';
import {checkNodeEntry} from './check-node-entry.cjs';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
import test from 'node:test';
import {mkdtempSync, mkdirSync, readFileSync, readdirSync, writeFileSync, unlinkSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath, pathToFileURL} from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const parent = join(repo, '.tmp', 'project-write');
mkdirSync(parent, {recursive: true});

function fixture() {
  const root = mkdtempSync(join(parent, 'case-'));
  const project = join(root, 'project');
  mkdirSync(join(project, 'sdk'), {recursive: true});
  writeFileSync(join(project, 'action.json'), JSON.stringify({schema_version: 1, actions: [{runtime: 'node'}]}));
  writeFileSync(join(project, 'main.mjs'), 'throw Error("AUTHOR CODE MUST NOT RUN");\n');
  writeFileSync(join(project, 'sdk', 'api.d.ts'), 'old API\n');
  writeFileSync(join(project, 'sdk', 'clipboard-history-api.d.ts'), 'old history API\n');
  return {root, project};
}

function refresh(project, flags = []) {
  return spawnSync(process.execPath, [...flags, join(repo, 'tools/script-project.mjs'),
    'refresh', '--runtime', 'node', '--project', project], {
    encoding: 'utf8', windowsHide: true, timeout: 60000,
  });
}

function portableFixture(layout) {
  const root=mkdtempSync(join(parent,'portable-'));
  const project=join(root,'project'),source=join(root,'source');mkdirSync(project);
  const prefix=layout==='sdk'?'sdk/':'';if(prefix)mkdirSync(join(project,'sdk'));
  mkdirSync(join(source,'crates/omnitrigger-actions/runtime'),{recursive:true});
  for(const name of ['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts','script-api.d.ts']) {
    const original=readFileSync(join(repo,'crates/omnitrigger-actions/runtime',name),'utf8');
    writeFileSync(join(source,'crates/omnitrigger-actions/runtime',name),original+'\n// target SDK revision fixture\n');
    if(name!=='script-api.d.ts'||prefix)writeFileSync(join(project,prefix+name),original);
  }
  const reference=prefix?'sdk/script-api.d.ts':'ambient.d.ts';
  if(!prefix)writeFileSync(join(project,'ambient.d.ts'),'/// <reference path="./script-types.d.ts" />\n');
  writeFileSync(join(project,'main.mjs'),`/// <reference path="./${reference}" />
throw Error('AUTHOR INITIALIZER MUST NOT RUN');
/** @type {ScriptMain} */
export function main(ctx) { return {processName:ctx.desktop.target().processName}; }
`);
  const actions=[{id:'script',runtime:'script',entry:'main.mjs'}];
  const files=['main.mjs',reference,prefix+'api.d.ts'];
  if(!prefix){
    actions.push({id:'node',runtime:'node',entry:'node.mjs'});files.push('node.mjs');
    writeFileSync(join(project,'node.mjs'),`throw Error('NODE AUTHOR MUST NOT RUN');
/** @param {import('./api.js').ActionContext} ctx */
export default async function main(ctx) { const target=await ctx.desktop.target();return {processName:target.processName}; }
`);
  }
  writeFileSync(join(project,'action.json'),JSON.stringify({schema_version:1,id:'synthetic.portable',actions}));
  writeFileSync(join(project,'jsconfig.json'),JSON.stringify({compilerOptions:{allowJs:true,checkJs:true,strict:true,noEmit:true,skipLibCheck:false,target:'ES2020',module:'ESNext',moduleResolution:'Bundler',lib:['ES2020'],types:[]},files}));
  writeFileSync(join(project,'README.md'),'作者说明不能改写\n');
  const authorFiles=['action.json','jsconfig.json','main.mjs','README.md',...(!prefix?['node.mjs','ambient.d.ts']:[])];
  return {project,source,prefix,authorFiles};
}
function refreshPortable(item) {
  return spawnSync(process.execPath,[join(repo,'tools/script-project.mjs'),'refresh','--runtime','script',
    '--project',item.project,'--source-root',item.source],{encoding:'utf8',windowsHide:true,timeout:60000});
}

test('canonical普通包两种布局更新SDK并保留作者文件，各自strict且不执行代码',()=>{
  for(const layout of ['sdk','flat']){
    const item=portableFixture(layout),before=item.authorFiles.map(name=>readFileSync(join(item.project,name)));
    const result=refreshPortable(item);assert.ifError(result.error);assert.equal(result.status,0,result.stderr);
    const receipt=JSON.parse(result.stdout);assert.equal(receipt.layout,layout);assert.equal(receipt.author_code_executed,false);
    assert.equal(receipt.author_files_changed,false);
    for(const [i,name] of item.authorFiles.entries())assert.deepEqual(readFileSync(join(item.project,name)),before[i]);
    for(const name of ['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts',...(item.prefix?['script-api.d.ts']:[])]){
      assert.deepEqual(readFileSync(join(item.project,item.prefix+name)),readFileSync(join(item.source,'crates/omnitrigger-actions/runtime',name)));
    }
    const config=ts.readConfigFile(join(item.project,'jsconfig.json'),ts.sys.readFile);assert.equal(config.error,undefined);
    const parsed=ts.parseJsonConfigFileContent(config.config,ts.sys,item.project);
    const diagnostics=ts.getPreEmitDiagnostics(ts.createProgram(parsed.fileNames,parsed.options));
    assert.equal(diagnostics.length,0,ts.formatDiagnosticsWithColorAndContext(diagnostics,{getCanonicalFileName:x=>x,getCurrentDirectory:()=>item.project,getNewLine:()=> '\n'}));
    if(layout==='sdk'){
      const again=refreshPortable(item);assert.ifError(again.error);assert.equal(again.status,0,again.stderr);
      for(const [i,name] of item.authorFiles.entries())assert.deepEqual(readFileSync(join(item.project,name)),before[i]);
    }
  }
});

test('canonical声明被作者修改或已识别布局缺件时，预检拒绝且不部分覆盖',()=>{
  const item=portableFixture('flat');const declarations=['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts'];
  writeFileSync(join(item.project,'script-types.d.ts'),readFileSync(join(item.project,'script-types.d.ts'),'utf8')+'\n// author edit\n');
  const before=[...item.authorFiles,...declarations].map(name=>[name,readFileSync(join(item.project,name))]);
  const changed=refreshPortable(item);assert.ifError(changed.error);assert.equal(changed.status,1);
  assert.equal(JSON.parse(changed.stderr).error.code,'project_sdk_unrecognized');
  for(const [name,bytes] of before)assert.deepEqual(readFileSync(join(item.project,name)),bytes);
  unlinkSync(join(item.project,'clipboard-history-api.d.ts'));
  const incomplete=refreshPortable(item);assert.ifError(incomplete.error);assert.equal(incomplete.status,1);
  assert.equal(JSON.parse(incomplete.stderr).error.code,'project_sdk_layout_incomplete');
  for(const [name,bytes] of before.filter(([name])=>name!=='clipboard-history-api.d.ts'))assert.deepEqual(readFileSync(join(item.project,name)),bytes);
});

test('canonical根目录声明与旧Script SDK并存时拒绝歧义且保留全部文件',()=>{
  const item=portableFixture('flat');mkdirSync(join(item.project,'sdk'));
  // 真实旧生成Script布局使用sdk/api.d.ts，不能与根目录SDK一起保留。
  writeFileSync(join(item.project,'sdk/api.d.ts'),readFileSync(join(repo,'crates/omnitrigger-actions/runtime/script-types.d.ts')));
  const names=[...item.authorFiles,'api.d.ts','script-types.d.ts','clipboard-history-api.d.ts','sdk/api.d.ts'];
  const before=names.map(name=>[name,readFileSync(join(item.project,name))]);
  const result=refreshPortable(item);assert.ifError(result.error);assert.equal(result.status,1);
  assert.equal(JSON.parse(result.stderr).error.code,'project_sdk_layout_ambiguous');
  for(const [name,bytes] of before)assert.deepEqual(readFileSync(join(item.project,name)),bytes);
});

test('实际刷新只更新SDK声明与入口检查器并保留作者文件', () => {
  const {project} = fixture();
  const before = ['action.json', 'main.mjs'].map(name => readFileSync(join(project, name)));
  const result = refresh(project);
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  const receipt = JSON.parse(result.stdout);
  assert.equal(receipt.status, 'refreshed');
  assert.equal(receipt.author_files_changed, false);
  assert.equal(receipt.author_code_executed, false);
  for (const [index, name] of ['action.json', 'main.mjs'].entries()) {
    assert.deepEqual(readFileSync(join(project, name)), before[index]);
  }
  for (const name of ['api.d.ts', 'clipboard-history-api.d.ts']) {
    assert.deepEqual(readFileSync(join(project, 'sdk', name)),
      readFileSync(join(repo, 'crates/omnitrigger-actions/runtime', name)));
  }
  assert.deepEqual(readFileSync(join(project,'sdk/check-node-entry.cjs')),
    readFileSync(join(repo,'tools/script-sdk-check/check-node-entry.cjs')));
  assert(readdirSync(join(project, 'sdk')).every(name => !name.endsWith('.tmp')));
});

test('临时文件碰撞拒绝刷新且不删除已有文件', () => {
  const {root, project} = fixture();
  const preload = join(root, 'collision.mjs');
  writeFileSync(preload, `import crypto from 'node:crypto';
import {syncBuiltinESMExports} from 'node:module';
crypto.randomUUID = () => 'collision';
syncBuiltinESMExports();\n`);
  const temporary = join(project, 'sdk', 'api.d.ts.sdk-collision.tmp');
  writeFileSync(temporary, 'EXISTING FILE MUST SURVIVE\n');
  const result = refresh(project, ['--import', pathToFileURL(preload).href]);
  assert.ifError(result.error);
  assert.equal(result.status, 1, result.stdout);
  assert.match(result.stderr, /^\s*\{/, result.stderr);
  assert.equal(JSON.parse(result.stderr).error.code, 'project_failed');
  assert.equal(readFileSync(temporary, 'utf8'), 'EXISTING FILE MUST SURVIVE\n');
  assert.equal(readFileSync(join(project, 'sdk', 'api.d.ts'), 'utf8'), 'old API\n');
  assert.equal(readFileSync(join(project, 'sdk', 'clipboard-history-api.d.ts'), 'utf8'), 'old history API\n');
});

test('Node 工程检查发现 tsc 漏掉的入口导出错误，且不执行作者初始化器',()=>{
  const parent=join(repo,'.tmp');mkdirSync(parent,{recursive:true});
  const root=mkdtempSync(join(parent,'node-entry-contract-'));mkdirSync(join(root,'sdk'));
  for(const name of ['api.d.ts','clipboard-history-api.d.ts']){
    writeFileSync(join(root,'sdk',name),readFileSync(join(repo,'crates/omnitrigger-actions/runtime',name)));
  }
  const manifest={schema_version:1,actions:[{id:'run',runtime:'node',entry:'main.mjs'}]};
  const config={compilerOptions:{allowJs:true,checkJs:true,strict:true,noEmit:true,target:'ES2022',module:'ESNext',moduleResolution:'Bundler',lib:['ES2022'],types:[]},include:['**/*.mjs','sdk/**/*.d.ts']};
  writeFileSync(join(root,'action.json'),JSON.stringify(manifest));writeFileSync(join(root,'jsconfig.json'),JSON.stringify(config));
  const entry=join(root,'main.mjs');
  const body=`throw new Error('检查绝不能执行作者代码');
/** @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx){
  const list=await ctx.fs.readDirectory('C:\\\\synthetic',{metadata:true,order:'native',entryType:'file',searchPattern:'*',matchMode:'windows-find'});
  /** @type {string} */ const ticks=list.entries[0].metadata.lastWriteTimeLocalTicks100ns;
  const written=await ctx.clipboard.writeFiles(['C:\\\\synthetic']);
  return {ticks,written};
}
`;
  function typeCheck(text){
    writeFileSync(entry,text);
    const converted=ts.parseJsonConfigFileContent(config,ts.sys,root);
    const diagnostics=ts.getPreEmitDiagnostics(ts.createProgram(converted.fileNames,converted.options));
    assert.equal(diagnostics.length,0,ts.formatDiagnostics(diagnostics,{getCanonicalFileName:x=>x,getCurrentDirectory:()=>root,getNewLine:()=> '\n'}));
    return checkNodeEntry({projectDirectory:root,typescript:ts});
  }
  const good=typeCheck(body);assert.equal(good.passed,true);assert.equal(good.authorCodeExecuted,false);
  const named=typeCheck(body.replace('export default async','export async'));
  assert.equal(named.passed,false);assert.equal(named.diagnostics[0].code,'node_entry_default_missing');
  const number=typeCheck('throw new Error("不得执行"); export default 42;\n');
  assert.equal(number.passed,false);assert.equal(number.diagnostics[0].code,'node_entry_default_noncallable');
  // 按当前清单检查入口，不能在改名后仍检查旧 main.mjs。
  writeFileSync(join(root,'renamed.mjs'),body);manifest.actions[0].entry='renamed.mjs';
  writeFileSync(join(root,'action.json'),JSON.stringify(manifest));
  assert.equal(checkNodeEntry({projectDirectory:root,typescript:ts}).passed,true);
});

test('Node 入口结果按真实宿主契约拒绝明确非 JSON 类型并保留动态未确认',async()=>{
  const root=mkdtempSync(join(parent,'node-result-contract-'));
  const config={compilerOptions:{allowJs:true,checkJs:true,strict:true,noEmit:true,target:'ES2022',module:'ESNext',moduleResolution:'Bundler',lib:['ES2022'],types:[]},include:['*.mjs']};
  writeFileSync(join(root,'action.json'),JSON.stringify({schema_version:1,actions:[{runtime:'node',entry:'main.mjs'}]}));
  writeFileSync(join(root,'jsconfig.json'),JSON.stringify(config));
  function inspect(body){
    writeFileSync(join(root,'main.mjs'),`throw new Error('不得执行作者初始化器');\n${body}\n`);
    const parsed=ts.parseJsonConfigFileContent(config,ts.sys,root);
    assert.equal(ts.getPreEmitDiagnostics(ts.createProgram(parsed.fileNames,parsed.options)).length,0);
    return checkNodeEntry({projectDirectory:root,typescript:ts});
  }
  const cases=[
    ['function','export default () => () => 1;',false],
    ['async-function','export default async () => () => 1;',false],
    ['promise-function','export default () => Promise.resolve(() => 1);',false],
    ['class-constructor','export default () => class Synthetic {};',false],
    ['symbol','export default () => Symbol("synthetic");',false],
    ['bigint','export default () => 1n;',false],
    ['undefined','export default () => undefined;',true],
    ['void','export default () => {};',true],
    ['nested-promise','export default () => Promise.resolve(Promise.resolve(1));',true],
    ['object-promise','export default () => ({value:Promise.resolve(1)});',true],
    ['object-function','export default () => ({value:() => 1});',true],
    ['unknown','/** @returns {unknown} */\nexport default () => JSON.parse("null");',true],
    ['mixed','/** @param {boolean} flag */\nexport default flag => flag ? 1 : () => 1;',true],
    ['function-toJSON','export default () => Object.assign(() => 1,{toJSON:() => 1});',true],
  ];
  const results=cases.map(([name,body,expected])=>({name,expected,result:inspect(body)}));
  console.log(JSON.stringify({nodeEntryResultCases:results.map(({name,result})=>({name,passed:result.passed,codes:result.diagnostics.map(item=>item.code)})),authorCodeExecuted:false}));
  for(const {name,expected,result} of results){
    assert.equal(result.passed,expected,name);
    assert.equal(result.authorCodeExecuted,false);
    if(!expected) assert.equal(result.diagnostics[0].code,'node_entry_result_nonjson',name);
    if(['unknown','mixed','function-toJSON'].includes(name)){
      assert.equal(result.diagnostics[0].code,'node_entry_result_unresolved',name);
      assert.equal(result.diagnostics[0].severity,'warning',name);
      assert.equal(result.runtimeVerified,false,name);
    }
  }
  // 与 host.mjs 的 output ?? null / JSON.stringify 对齐；仅验证合成值，不调用动作模块。
  const response=value=>JSON.parse(JSON.stringify({api_version:1,invocation_id:'synthetic',status:'ok',output:value??null}));
  assert.equal(response(undefined).output,null);
  assert.equal(response(await Promise.resolve(Promise.resolve(1))).output,1);
  assert.deepEqual(response({value:Promise.resolve(1)}).output,{value:{}});
  assert.deepEqual(response({value:()=>1}).output,{});
  assert.equal(Object.hasOwn(response(()=>1),'output'),false);
  assert.equal(Object.hasOwn(response(Symbol('synthetic')),'output'),false);
  assert.throws(()=>response(1n),TypeError);
});

// create诊断合同：只启动Node前端，应用与Windows metadata均由--import替身接管。
function createFailureFixture(mode, options={}) {
  const root=mkdtempSync(join(parent,'create-diagnostic-'));
  const config=join(root,'profile/bindings.json'),source=join(root,'source');
  const project=join(dirname(config),'actions','author-cli-case');
  mkdirSync(join(project,'sdk'),{recursive:true});
  mkdirSync(join(source,'crates/omnitrigger-actions/runtime'),{recursive:true});
  writeFileSync(config,'{}\n');
  writeFileSync(join(source,'crates/omnitrigger-actions/runtime/script-types.d.ts'),'source declaration fixture\n');
  const binary=join(root,'not-an-application.exe');writeFileSync(binary,'TEST PLACEHOLDER; NEVER EXECUTE\n');
  writeFileSync(join(project,'main.mjs'),'throw Error("AUTHOR CODE MUST NOT RUN");\n');
  writeFileSync(join(project,'author-note.txt'),'部分工程必须保留\n');
  writeFileSync(join(project,'sdk/api.d.ts'),'binary declaration fixture\n');
  const authorFiles=['main.mjs','author-note.txt','sdk/api.d.ts'];
  const before=authorFiles.map(name=>readFileSync(join(project,name)));
  const names=readdirSync(project).sort(),sdkNames=readdirSync(join(project,'sdk')).sort();
  const traceFile=join(root,'trace.json'),preload=join(root,'creator-substitute.mjs');
  const setup={mode,binary,project,traceFile,untrusted:join(root,'untrusted-reply'),...options};
  writeFileSync(preload,`import childProcess from 'node:child_process';
import {writeFileSync} from 'node:fs';
import {syncBuiltinESMExports} from 'node:module';
const setup=${JSON.stringify(setup)};
const trace={application_spawns:0,metadata_spawns:0};
process.once('exit',()=>writeFileSync(setup.traceFile,JSON.stringify(trace)));
childProcess.spawnSync=(program,args,options)=>{
  if(program==='powershell.exe'){
    if(options.timeout!==15000)throw Error('METADATA BUDGET CHANGED');
    trace.metadata_spawns++;trace.metadata_timeout_ms=options.timeout;return {status:0,stdout:'',stderr:''};
  }
  if(program!==setup.binary)throw Error('UNEXPECTED CHILD PROCESS');
  trace.application_spawns++;
  Object.assign(trace,{args,timeout:options.timeout,maxBuffer:options.maxBuffer,windowsHide:options.windowsHide});
  const began=performance.now();
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0,25);
  const reply={status:'created',project:{package_root:setup.mode==='wrong-root'?setup.untrusted:setup.project}};
  const result={status:0,signal:null,stdout:JSON.stringify(reply),stderr:'CREATOR_STDERR_SECRET'};
  if(setup.mode==='spawn'){
    result.error=Object.assign(Error('CREATOR_ERROR_SECRET'),{code:setup.cause});
    result.status=setup.apparentSuccess?0:null;result.signal=setup.signal??'SIGTERM';
    result.stdout='CREATOR_STDOUT_SECRET';
  }else if(setup.mode==='invalid-json')result.stdout='CREATOR_STDOUT_SECRET';
  trace.spawn_elapsed_ms=performance.now()-began;
  return result;
};
syncBuiltinESMExports();
`);
  return {root,config,source,project,binary,preload,traceFile,options,assertPreserved(){
    assert.deepEqual(readdirSync(project).sort(),names);
    assert.deepEqual(readdirSync(join(project,'sdk')).sort(),sdkNames);
    for(const[i,name]of authorFiles.entries())assert.deepEqual(readFileSync(join(project,name)),before[i],name);
  }};
}

function inspectCreateFailure(item) {
  const began=performance.now();
  const result=spawnSync(process.execPath,['--import',pathToFileURL(item.preload).href,
    join(repo,'tools/script-project.mjs'),'create','--runtime','node','--binary',item.binary,
    '--config',item.config,'--id',item.options.id??'author-cli-case','--source-root',item.source],
    {encoding:'utf8',windowsHide:true,timeout:60000});
  const elapsed=performance.now()-began;
  assert.ifError(result.error);assert.equal(result.status,1,result.stdout+result.stderr);
  assert.equal(result.stdout,'');
  const reply=JSON.parse(result.stderr),trace=JSON.parse(readFileSync(item.traceFile,'utf8'));
  assert.equal(reply.status,'failed');assert.equal(reply.author_code_executed,false);
  assert.equal(reply.error.project_state,'unconfirmed');
  assert.equal(trace.application_spawns,1,'不自动重试应用创建');
  assert.equal(trace.timeout,60000);assert.equal(trace.maxBuffer,4*1024*1024);assert.equal(trace.windowsHide,true);
  if(process.platform==='win32')assert.equal(trace.metadata_timeout_ms,15000);
  assert.deepEqual(trace.args,['--create-script-action',item.options.id??'author-cli-case','--config',item.config]);
  assert.equal(reply.error.cli_timeout_ms,60000);
  assert(Number.isSafeInteger(reply.error.cli_elapsed_ms));
  assert(reply.error.cli_elapsed_ms>=trace.spawn_elapsed_ms-1,'elapsed必须包含替身内实际有界等待');
  assert(reply.error.cli_elapsed_ms<=elapsed+2,'elapsed不能伪装为配置超时值');
  assert(!('application_created'in reply));assert(!('files_modified'in reply));assert(!('effect_state'in reply.error));
  assert.notEqual(reply.error.files_modified,false,'失败不能声称没有写入');
  for(const secret of ['CREATOR_STDOUT_SECRET','CREATOR_STDERR_SECRET','CREATOR_ERROR_SECRET'])assert(!result.stderr.includes(secret));
  item.assertPreserved();
  return reply.error;
}

test('create失败诊断保留子进程错误、真实elapsed与部分工程，应用只尝试一次',()=>{
  for(const cause of ['ETIMEDOUT','ENOENT','EACCES','ENOBUFS']){
    const item=createFailureFixture('spawn',{cause,apparentSuccess:cause==='ETIMEDOUT'});
    const error=inspectCreateFailure(item);
    assert.equal(error.code,'project_cli_unavailable');assert.equal(error.stage,'application_create');
    assert.equal(error.spawn_error_code,cause);assert.equal(error.project_root_to_check,item.project);
    assert.equal(error.cli_exit_code,cause==='ETIMEDOUT'?0:null);assert.equal(error.cli_signal,'SIGTERM');
  }
});

test('create失败诊断不建议越界路径，并限制错误码与信号字段',()=>{
  const escaped=createFailureFixture('spawn',{cause:'ETIMEDOUT',id:'../escape'});
  const unsafe=inspectCreateFailure(escaped);
  assert.equal(unsafe.stage,'application_create');assert(!('project_root_to_check'in unsafe));
  const bounded=createFailureFixture('spawn',{cause:'X'.repeat(65),signal:'SIGTERM\nCREATOR_STDERR_SECRET'});
  const error=inspectCreateFailure(bounded);
  assert.equal(error.spawn_error_code,'unknown');assert.equal(error.cli_signal,null);
  assert.equal(error.project_root_to_check,bounded.project);
});

test('create失败诊断区分回复和SDK阶段，不泄露回复或采用未核对路径',()=>{
  for(const[mode,code,stage]of[
    ['invalid-json','project_failed','application_reply'],
    ['wrong-root','project_cli_invalid_reply','application_reply'],
    ['sdk-mismatch','project_sdk_binary_mismatch','sdk_validation'],
  ]){
    const item=createFailureFixture(mode),error=inspectCreateFailure(item);
    assert.equal(error.code,code);assert.equal(error.stage,stage);
    assert.equal(error.project_root_to_check,item.project);
    assert(!JSON.stringify(error).includes(join(item.root,'untrusted-reply')));
    assert.equal(error.cli_exit_code,0);assert.equal(error.cli_signal,null);
    assert(!('spawn_error_code'in error));
  }
});

test('canonical快照指纹覆盖当前来源与已携带声明的LF和CRLF，保留后续刷新链',()=>{
  const source=ts.createSourceFile('script-project.mjs',readFileSync(join(repo,'tools/script-project.mjs'),'utf8'),
    ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);
  let initializer;
  for(const statement of source.statements)if(ts.isVariableStatement(statement)){
    for(const declaration of statement.declarationList.declarations)if(declaration.name.getText(source)==='canonicalSdkHashes')initializer=declaration.initializer;
  }
  assert(initializer&&ts.isObjectLiteralExpression(initializer),'复用现有canonical表，不执行创建/刷新工具');
  const known=new Map(initializer.properties.map(property=>{
    assert(ts.isPropertyAssignment(property)&&ts.isStringLiteral(property.name)&&ts.isArrayLiteralExpression(property.initializer));
    const hashes=property.initializer.elements.map(value=>{assert(ts.isStringLiteral(value)&&/^[0-9a-f]{64}$/.test(value.text));return value.text;});
    assert.equal(new Set(hashes).size,hashes.length,'历史指纹追加且不重复');
    return[property.name.text,hashes];
  }));
  const groups=[
    ['crates/omnitrigger-actions/runtime',['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts','script-api.d.ts']],
    ['examples/actions/mixed-projects/sdk',['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts']],
    ['examples/actions/source-file-preview/sdk',['api.d.ts','clipboard-history-api.d.ts']],
  ];
  for(const[directory,names]of groups)for(const name of names){
    assert(known.has(name),`${name}须登记在现有canonical表`);
    const lf=readFileSync(join(repo,directory,name),'utf8').replaceAll('\r\n','\n');
    for(const[ending,text]of[['LF',lf],['CRLF',lf.replaceAll('\n','\r\n')]]){
      const digest=createHash('sha256').update(text,'utf8').digest('hex');
      assert(known.get(name).includes(digest),`${directory}/${name} ${ending}指纹未登记：${digest}；SDK更新须追加已核验指纹并保留历史值`);
    }
  }
});
