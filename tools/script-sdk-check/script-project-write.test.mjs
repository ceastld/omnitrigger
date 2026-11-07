import ts from 'typescript';
import {checkNodeEntry} from './check-node-entry.cjs';
import assert from 'node:assert/strict';
import test from 'node:test';
import {mkdtempSync, mkdirSync, readFileSync, readdirSync, writeFileSync} from 'node:fs';
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
