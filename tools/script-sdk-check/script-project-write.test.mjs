import ts from 'typescript';
import {checkNodeEntry} from './check-node-entry.cjs';
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
