// 只执行从实际宿主源码提取的纯诊断函数/错误对象表达式，不启动宿主或作者模块。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {basename,dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import vm from 'node:vm';
import test from 'node:test';
import ts from 'typescript';

const file=fileURLToPath(import.meta.url);
const root=resolve(dirname(file),'../..');
const source=ts.createSourceFile('host.mjs',readFileSync(resolve(root,'crates/omnitrigger-actions/runtime/host.mjs'),'utf8'),ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);
let declaration,envelope;
const property=(object,name)=>object.properties.find(node=>ts.isPropertyAssignment(node)&&node.name.getText(source)===name);
function visit(node) {
  if(ts.isFunctionDeclaration(node)&&node.name?.text==='errorDiagnostic')declaration=node;
  if(ts.isCallExpression(node)&&ts.isIdentifier(node.expression)&&node.expression.text==='respond'
      &&node.arguments[0]&&ts.isObjectLiteralExpression(node.arguments[0])) {
    const object=node.arguments[0];
    if(property(object,'status')?.initializer.getText(source)==="'error'")envelope=object;
  }
  ts.forEachChild(node,visit);
}
visit(source);assert(declaration&&envelope);
const render=vm.runInNewContext('('+declaration.getText(source)+')');
const response=vm.compileFunction('return ('+envelope.getText(source)+');',['error','errorDiagnostic']);

test('实际Rust模板拼接后的完整Node宿主只做语法检查，不启动宿主',()=>{
  const rustPath=resolve(root,'crates/omnitrigger-actions/src/node.rs');
  const rust=readFileSync(rustPath,'utf8');
  const template=rust.match(/format!\(\s*("(?:[^"\\]|\\.)*\{HOST\}(?:[^"\\]|\\.)*")\s*\)/u);
  assert(template);
  const combined=JSON.parse(template[1]).replace(/\{([A-Z_]+)\}/gu,(_,name)=>{
    const declaration=rust.match(new RegExp('const\\s+'+name+'\\s*:\\s*&str\\s*=\\s*include_str!\\("([^"\\n]+)"\\);','u'));
    assert(declaration,name);return readFileSync(resolve(dirname(rustPath),declaration[1]),'utf8');
  });
  const result=spawnSync(process.execPath,['--check','--input-type=module'],{
    input:combined,encoding:'utf8',windowsHide:true,timeout:15000,
  });
  assert.ifError(result.error);assert.equal(result.status,0,result.stderr);
});

test('Node诊断保留真实Error栈中的源文件位置，不执行宿主或作者代码',()=>{
  const error=new Error('NODE_LOCATION_SAMPLE');const stack=error.stack;
  assert(stack.includes(basename(file)));assert.equal(render(error),stack);
  assert.equal(render({message:'plain fault'}),'plain fault');
  assert.equal(render({message:'plain fault',stack:42}),'plain fault');
  assert.equal(render('plain value'),'plain value');
});

test('Node长诊断保留首尾源帧，Unicode及JSON输出不突破原响应边界',()=>{
  const error=new Error('字😀'.repeat(20000));const text=render(error);
  assert(text.length<=4096);assert(Buffer.byteLength(text,'utf8')<=16*1024);
  assert(text.includes('诊断已截断'));assert(text.includes(basename(file)));
  assert.equal(text,Buffer.from(text,'utf8').toString('utf8'));
  assert(Buffer.byteLength(JSON.stringify(response(error,render)),'utf8')<64*1024);
  assert.equal(render({message:'\uD800'}),'\uFFFD');
});

test('读取stack失败回退原消息，保留既有ActionError字段且不添加调试协议',()=>{
  const error=Object.assign(new Error('NATIVE_FIELDS_SAMPLE'),{
    code:'permission_required',effectState:'not_started',method:'bookmarks.getTree',requiredPermissions:['bookmarks'],
  });
  Object.defineProperty(error,'stack',{get(){throw new Error('STACK_GETTER_MUST_NOT_ESCAPE');}});
  const result=response(error,render);
  assert.equal(result.status,'error');assert.equal(result.error.message,'NATIVE_FIELDS_SAMPLE');
  assert.equal(result.error.code,'permission_required');assert.equal(result.error.effect_state,'not_started');
  assert.equal(result.error.method,'bookmarks.getTree');assert.deepEqual(result.error.required_permissions,['bookmarks']);
  assert.deepEqual(Object.keys(result.error).sort(),['code','effect_state','message','method','required_permissions']);
});
