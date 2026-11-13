// 分析SDK装配与VM全局工厂；不求值SDK、夹具或作者入口。
import assert from 'node:assert/strict';
import {readFileSync,readdirSync} from 'node:fs';
import {dirname,basename,relative,resolve,isAbsolute} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import ts from 'typescript';

const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const own=relative(repo,fileURLToPath(import.meta.url)).replaceAll('\\','/');
const read=file=>readFileSync(resolve(repo,file),'utf8');
const engineFile='tools/js-engine-probe/src/main.rs',engineSource=read(engineFile);
const bindings=new Map([...engineSource.matchAll(/const\s+(\w+):\s*&str\s*=\s*include_str!\("([^"]+)"\);/g)].map(x=>[x[1],x[2]]));
const format=engineSource.match(/let\s+source\s*=\s*format!\(\s*"((?:\\[\s\S]|[^"\\])*)"/)?.[1];
assert(format,'必须定位实际Script宿主source格式表达式');
const parts=[...format.matchAll(/\{([A-Z_]+)\}/g)].filter(x=>bindings.has(x[1])).map(x=>{
  const file=resolve(repo,dirname(engineFile),bindings.get(x[1])),local=relative(repo,file);
  assert(!isAbsolute(local)&&!local.startsWith('..'),'SDK源必须位于仓库内');return {symbol:x[1],file};
});
assert.equal(parts.at(-1)?.symbol,'SDK');
const sdkPart=parts.at(-1),sdk=readFileSync(sdkPart.file,'utf8');
const moduleText=new Map(parts.map(x=>[basename(x.file),readFileSync(x.file,'utf8')]));

function program(file,source){
  const options={allowJs:true,noLib:true,noResolve:true,types:[],target:ts.ScriptTarget.Latest,module:ts.ModuleKind.ESNext};
  const ast=ts.createSourceFile(file,source,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);
  assert.equal(ast.parseDiagnostics.length,0,`${file}源码必须可解析`);
  const host={getSourceFile:name=>name===file?ast:undefined,getDefaultLibFileName:()=>'',writeFile:()=>{},getCurrentDirectory:()=>repo,getDirectories:()=>[],fileExists:name=>name===file,readFile:name=>name===file?source:undefined,getCanonicalFileName:x=>x,useCaseSensitiveFileNames:()=>true,getNewLine:()=> '\n'};
  const value=ts.createProgram([file],options,host);return {ast:value.getSourceFile(file),checker:value.getTypeChecker()};
}
function requiredFactories(source){
  const {ast}=program('sdk.js',source),names=new Set();
  function visit(node){if(ts.isCallExpression(node)&&ts.isIdentifier(node.expression)&&/^create[A-Z].*Api$/.test(node.expression.text))names.add(node.expression.text);ts.forEachChild(node,visit);}
  visit(ast);return [...names];
}
const required=requiredFactories(sdk);
function missingFactories(modules,mocks,requiredNames=required,sdkSource=sdk){
  const prefix=modules.filter(name=>name!==basename(sdkPart.file)).map(name=>moduleText.get(name)??'').join('\n');
  const fake=[...mocks].map(name=>`var ${name};`).join('\n');
  const {ast,checker}=program('assembled.js',prefix+'\n'+fake+'\n'+sdkSource),missing=new Set();
  function visit(node){if(ts.isCallExpression(node)&&ts.isIdentifier(node.expression)&&requiredNames.includes(node.expression.text)&&!checker.getSymbolAtLocation(node.expression))missing.add(node.expression.text);ts.forEachChild(node,visit);}
  visit(ast);return [...missing];
}

function inspect(file,text){
  const {ast,checker}=program(file,text),sites=[];
  function declaration(node){const symbol=checker.getSymbolAtLocation(node);return symbol?.valueDeclaration??symbol?.declarations?.[0];}
  function init(node){
    if(!node)return undefined;
    if(ts.isIdentifier(node)){const d=declaration(node);return d&&'initializer' in d?d.initializer:d&&ts.isFunctionDeclaration(d)?d:undefined;}
    return node;
  }
  function callable(node,seen=new Set()){
    if(!node||seen.has(node))return false;seen.add(node);
    if(ts.isArrowFunction(node)||ts.isFunctionExpression(node)||ts.isFunctionDeclaration(node))return true;
    if(ts.isParenthesizedExpression(node))return callable(node.expression,seen);
    if(ts.isIdentifier(node))return callable(init(node),seen);
    return false;
  }
  function modules(node,seen=new Set()){
    if(!node)return [];
    if(ts.isIdentifier(node)){
      const d=declaration(node);if(!d||seen.has(d)||!('initializer' in d)||!d.initializer)return [];
      const next=new Set(seen);next.add(d);return modules(d.initializer,next);
    }
    if(ts.isStringLiteral(node)||ts.isNoSubstitutionTemplateLiteral(node)){
      const name=node.text.replaceAll('\\','/').split('/').at(-1);return moduleText.has(name)?[name]:[];
    }
    if(ts.isArrowFunction(node)||ts.isFunctionExpression(node))return [];
    if(ts.isCallExpression(node)){
      if(ts.isPropertyAccessExpression(node.expression)&&['slice','match','split','substring'].includes(node.expression.name.text))return [];
      const out=[];
      if(ts.isPropertyAccessExpression(node.expression))out.push(...modules(node.expression.expression,seen));
      for(const argument of node.arguments)out.push(...modules(argument,seen));return out;
    }
    const out=[];ts.forEachChild(node,child=>{out.push(...modules(child,seen));});return out;
  }
  function mockFactories(node,seen=new Set()){
    if(!node||seen.has(node))return new Set();seen.add(node);
    if(ts.isIdentifier(node))return mockFactories(init(node),seen);
    if(ts.isCallExpression(node)&&/createContext$/.test(node.expression.getText(ast)))return mockFactories(node.arguments[0],seen);
    const out=new Set();
    if(ts.isObjectLiteralExpression(node))for(const property of node.properties){
      if(ts.isSpreadAssignment(property)){for(const name of mockFactories(property.expression,seen))out.add(name);continue;}
      const name=property.name&&(ts.isIdentifier(property.name)||ts.isStringLiteral(property.name))?property.name.text:null;
      if(name&&/^create[A-Z][\w$]*Api$/.test(name)&&(ts.isMethodDeclaration(property)
          ||ts.isPropertyAssignment(property)&&callable(property.initializer)
          ||ts.isShorthandPropertyAssignment(property)&&callable(property.name)))out.add(name);
    }
    return out;
  }
  function visit(node){
    if(ts.isCallExpression(node)&&/(?:^|\.)(?:runInContext|runInNewContext|runInThisContext)$/.test(node.expression.getText(ast))&&node.arguments.length){
      const dependencies=modules(node.arguments[0]);
      if(dependencies.includes(basename(sdkPart.file))){
        const mocks=mockFactories(node.arguments[1]),location=ast.getLineAndCharacterOfPosition(node.getStart(ast));
        const end=dependencies.indexOf(basename(sdkPart.file)),prefix=dependencies.slice(0,end+1),late=dependencies.slice(end+1);
        sites.push({file,line:location.line+1,modules:[...new Set(prefix)],lateModules:late,mocks:[...mocks],missing:missingFactories(prefix,mocks)});
      }
    }
    ts.forEachChild(node,visit);
  }
  visit(ast);return sites;
}

function files(folder){
  const out=[];
  for(const item of readdirSync(resolve(repo,folder),{withFileTypes:true})){
    if(item.isSymbolicLink()||['node_modules','target'].includes(item.name)||/^(OneDrive|iCloud)|^Dropbox$/i.test(item.name))continue;
    const file=folder+'/'+item.name;
    if(item.isDirectory())out.push(...files(file));else if(/\.(?:mjs|cjs|js)$/.test(item.name)&&file!==own)out.push(file);
  }
  return out;
}
const candidates=[...files('tests'),...files('examples/actions'),...files('crates/omnitrigger-actions/runtime')];
const sites=candidates.flatMap(file=>inspect(file,read(file)));
const knownFiles=[
  'tests/clipboard_data_api.test.mjs','tests/desktop_captured_target.test.mjs',
  'tests/fs_read_directory_api.test.mjs','tests/fs_stat_api.test.mjs',
  'tests/process_capture_api.test.mjs','tests/process_capture_bridge_transport.test.mjs',
  'tests/process_capture_sdk.test.mjs','tests/raishot_resident_sdk.test.mjs',
  'tests/selection_max_units_sdk.test.mjs','tests/selection_read_sdk.test.mjs',
  'tests/source_file_preview.test.mjs','tests/ui_notify_api.test.mjs','tests/win32_edit_api.test.mjs',
  'examples/actions/process-output/cross-entry-contract.test.mjs','examples/actions/process-output/recipes.test.mjs',
  'examples/actions/script-choice/contract.test.mjs','examples/actions/selection-line-cleanup/recipes.test.mjs',
  'examples/actions/win32-edit-line-cleanup/recipes.test.mjs','crates/omnitrigger-actions/runtime/actions-api.test.mjs',
];

test('实际Script宿主为SDK所需工厂提供真实源定义',()=>{
  assert(required.length>0);assert.deepEqual(missingFactories(parts.map(x=>basename(x.file)),new Set()),[]);
});
test('完整Script SDK的第一方VM消费点均有真实工厂或明确全局替身',()=>{
  assert(sites.length>0,'必须发现完整SDK消费点');
  for(const file of knownFiles)assert(sites.some(site=>site.file===file),`已知完整SDK消费点不能漏扫：${file}`);
  for(const site of sites){
    assert.deepEqual(site.lateModules,[],`${site.file}:${site.line}工厂模块应在SDK构造前装配`);
    assert.deepEqual(site.missing,[],`${site.file}:${site.line} 缺工厂；源=${site.modules.join(',')} 替身=${site.mocks.join(',')}`);
  }
});
test('替身只来自实际VM上下文，必须明确提供函数而不是字段名或getter',()=>{
  const props=required.map(name=>`${name}:()=>({})`).join(',');
  const prefix="const sdk=readFileSync('tools/js-engine-probe/src/sdk.js','utf8');\n";
  const valid=inspect('synthetic.mjs',prefix+`const context={${props}};vm.runInContext(sdk,context);`);
  assert.equal(valid.length,1);assert.deepEqual(valid[0].missing,[]);
  for(const replacement of [`${required[0]}:undefined`,`get ${required[0]}(){return ()=>({});}`]){
    const bad=inspect('synthetic.mjs',prefix+`const context={${props.replace(`${required[0]}:()=>({})`,replacement)}};vm.runInContext(sdk,context);`);
    assert(bad[0].missing.includes(required[0]));
  }
  const unrelated=inspect('synthetic.mjs',prefix+`const other={${props}};const context={};vm.runInContext(sdk,context);`);
  assert.deepEqual(new Set(unrelated[0].missing),new Set(required));
});
test('局部API工厂和SDK切片不按完整SDK检查',()=>{
  const source="const sdk=readFileSync('tools/js-engine-probe/src/sdk.js','utf8');const api=readFileSync('desktop-api.js','utf8');vm.runInContext(api+';createDesktopApi',{});vm.runInContext(sdk.slice(0,40),{});";
  assert.deepEqual(inspect('fragments.mjs',source),[]);
});
test('新状态工厂出现时旧SDK装配不能因同文件其他工厂而蒙混通过',()=>{
  assert(sites.length>0);
  const changed=sdk+'\nctx.futureState = createFutureStateApi();',names=requiredFactories(changed);
  assert(names.includes('createFutureStateApi'));
  for(const site of sites)assert(missingFactories(site.modules,new Set(site.mocks),names,changed).includes('createFutureStateApi'));
});

// 只供隔离验证查看覆盖范围；不会求值被分析的源。
if(process.env.OMNITRIGGER_SCRIPT_ASSEMBLY_REPORT==='1')process.stdout.write(JSON.stringify({required,sites})+'\n');
