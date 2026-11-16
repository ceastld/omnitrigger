// 只读真实拼接源码并做语法检查；不导入测试夹具、运行宿主或动作入口。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {dirname,basename,relative,resolve,isAbsolute} from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import ts from 'typescript';

const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const read=file=>readFileSync(resolve(repo,file),'utf8');
const nodeRuntimeFile='crates/omnitrigger-actions/src/node.rs';
const points=[
  {file:'tests/ui_notify_api.test.mjs',kind:'call',name:'writeFileSync',first:'host'},
  {file:'tests/fs_stat_api.test.mjs',kind:'call',name:'writeFileSync',first:'hostPath'},
  {file:'examples/actions/script-choice/contract.test.mjs',kind:'call',name:'writeFileSync',first:'host'},
  {file:'tests/browser_workflows.test.mjs',kind:'variable',name:'sources'},
  {file:'tests/native_parameters_snapshot.test.mjs',kind:'variable',name:'source'},
  {file:'scripts/test-browser-actions.mjs',kind:'variable',name:'host'},
];

function runtimeParts(source){
  const bindings=new Map([...source.matchAll(/const\s+(\w+):\s*&str\s*=\s*include_str!\("([^"]+)"\);/g)].map(x=>[x[1],x[2]]));
  const format=source.match(/let\s+host_source\s*=\s*format!\(\s*"([^"\n]*)"\s*\)/)?.[1];
  assert(format,'找不到实际NodeRuntime宿主format表达式');
  const parts=[...format.matchAll(/\{([A-Z_]+)\}/g)].map(x=>{
    assert(bindings.has(x[1]),`缺少include_str绑定：${x[1]}`);
    const file=resolve(repo,dirname(nodeRuntimeFile),bindings.get(x[1])),local=relative(repo,file);
    assert(!isAbsolute(local)&&!local.startsWith('..'),'宿主模块必须位于仓库内');
    return {symbol:x[1],name:basename(file),file};
  });
  assert.equal(parts.at(-1)?.symbol,'HOST');
  assert.equal(new Set(parts.map(x=>x.name)).size,parts.length,'宿主模块不能重名或重复');
  return parts;
}
const runtimeSource=read(nodeRuntimeFile),canonical=runtimeParts(runtimeSource);

function parse(file,source){
  const ast=ts.createSourceFile(file,source,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);
  assert.equal(ast.parseDiagnostics.length,0,`${file}解析失败`);return ast;
}

function composition(point,source,parts=canonical){
  const ast=parse(point.file,source),variables=new Map(),candidates=[];
  function visit(node){
    if(ts.isVariableDeclaration(node)&&ts.isIdentifier(node.name)&&node.initializer){
      if(!variables.has(node.name.text))variables.set(node.name.text,node.initializer);
      if(point.kind==='variable'&&node.name.text===point.name)candidates.push(node.initializer);
    }
    if(point.kind==='call'&&ts.isCallExpression(node)&&node.expression.getText(ast)===point.name
        &&node.arguments[0]?.getText(ast)===point.first&&node.arguments.length>=2)candidates.push(node.arguments[1]);
    ts.forEachChild(node,visit);
  }
  visit(ast);
  const names=new Set(parts.map(x=>x.name));
  function dependencies(node,seen=new Set()){
    if(ts.isStringLiteral(node)||ts.isNoSubstitutionTemplateLiteral(node)){
      const name=node.text.replaceAll('\\','/').split('/').at(-1);return names.has(name)?[name]:[];
    }
    if(ts.isIdentifier(node)&&variables.has(node.text)&&!seen.has(node.text)){
      const next=new Set(seen);next.add(node.text);return dependencies(variables.get(node.text),next);
    }
    const result=[];ts.forEachChild(node,child=>{result.push(...dependencies(child,seen));});return result;
  }
  const actual=candidates.map(node=>({node,names:[...dependencies(node)]})).filter(x=>x.names.includes('host.mjs'));
  assert.equal(actual.length,1,`${point.file}必须定位到唯一完整宿主表达式`);
  const expected=parts.map(x=>x.name),missing=expected.filter(x=>!actual[0].names.includes(x));
  assert.equal(missing.length,0,`${point.file}宿主缺模块：${missing.join(', ')}`);
  assert.deepEqual(actual[0].names,expected,`${point.file}必须沿实际NodeRuntime顺序装配全部模块`);
  return actual[0].names;
}

function checkHost(names){
  const assembled=names.map(name=>readFileSync(canonical.find(x=>x.name===name).file,'utf8')).join('\n');
  const ast=parse('assembled-host.mjs',assembled),declarations=new Map();
  const declare=name=>declarations.set(name,(declarations.get(name)??0)+1);
  ts.forEachChild(ast,node=>{
    if(ts.isFunctionDeclaration(node)&&node.name)declare(node.name.text);
    if(ts.isVariableStatement(node))for(const item of node.declarationList.declarations)if(ts.isIdentifier(item.name))declare(item.name.text);
  });
  const host=parse('host.mjs',readFileSync(canonical.at(-1).file,'utf8')),required=new Set();
  function visit(node){
    if(ts.isCallExpression(node)&&ts.isIdentifier(node.expression)&&/^create[A-Z].*Api$/.test(node.expression.text))required.add(node.expression.text);
    ts.forEachChild(node,visit);
  }
  visit(host);
  for(const name of required)assert.equal(declarations.get(name),1,`工厂必须恰有一个真实定义：${name}`);
  const result=spawnSync(process.execPath,['--input-type=module','--check'],{input:assembled,encoding:'utf8',windowsHide:true,timeout:10000});
  assert.ifError(result.error);assert.equal(result.status,0,result.stderr);
}

test('产品NodeRuntime的真实include_str与format完整装配可解析',()=>checkHost(canonical.map(x=>x.name)));
for(const point of points)test(`实际Node宿主拼接：${point.file}`,()=>checkHost(composition(point,read(point.file))));

test('同文件Script SDK带actions不能掩盖Node宿主缺模块',()=>{
  const point=points[0];let source=read(point.file),ast=parse(point.file,source),history,sdk;
  function find(node){
    if(ts.isVariableDeclaration(node)&&node.name.getText(ast)==='historySource')history=node.initializer;
    ts.forEachChild(node,find);
  }
  find(ast);assert(history);
  // 复现旧错误：仅从Node使用的historySource中移除actions，另一路sdkSource仍加载它。
  const before=history.getText(ast),without=before.replace(/\s*\+\s*readFileSync\(join\(runtime,\s*['"]actions-api\.js['"]\),\s*['"]utf8['"]\)/,'');
  assert.notEqual(before,without);source=source.slice(0,history.getStart(ast))+without+source.slice(history.end);
  ast=parse(point.file,source);
  function findSdk(node){if(ts.isVariableDeclaration(node)&&node.name.getText(ast)==='sdkSource')sdk=node.initializer;ts.forEachChild(node,findSdk);}
  findSdk(ast);assert(sdk);
  source=source.slice(0,sdk.getStart(ast))+"readFileSync(join(runtime,'actions-api.js'),'utf8')+'\\n'+"+sdk.getText(ast)+source.slice(sdk.end);
  assert.throws(()=>composition(point,source),/宿主缺模块：actions-api\.js/);
});

test('产品以后增加工厂模块时六条消费路径也必须更新',()=>{
  const future='const FUTURE_API: &str = include_str!("../runtime/future-api.js");\n'
    +runtimeSource.replace('{HOST}','{FUTURE_API}\\n{HOST}');
  const parts=runtimeParts(future);
  for(const point of points)assert.throws(()=>composition(point,read(point.file),parts),/宿主缺模块：[^\n]*future-api\.js/);
});

test('完整宿主不能重复拼入同一个工厂模块',()=>{
  const point=points[0],source=read(point.file),needle="readFileSync(join(runtime, 'actions-api.js'), 'utf8')";
  assert(source.includes(needle));
  assert.throws(()=>composition(point,source.replace(needle,needle+' + '+needle)),/必须沿实际NodeRuntime顺序装配全部模块/);
});
