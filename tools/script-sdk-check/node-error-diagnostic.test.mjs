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

test('结构化本机错误保留精确消息且不读取宿主栈，普通Node错误仍保留源码',()=>{
  for(const fields of [{code:'ui_busy',method:'ui.notify',effectState:'not_started'},
    {code:'cancelled',effectState:'unknown'},{code:'fs_error',method:'fs.stat'}]) {
    let stackReads=0;const error=Object.assign(new Error('未确认'),fields);
    Object.defineProperty(error,'stack',{get(){stackReads++;return 'HOST IMPLEMENTATION STACK';}});
    assert.equal(render(error),'未确认');assert.equal(stackReads,0);
    assert.equal(response(error,render).error.message,'未确认');
  }
  const ordinary=Object.assign(new Error('NODE_SOURCE_FRAME'),{code:'ERR_SYNTHETIC'});
  assert.equal(render(ordinary),ordinary.stack);assert(render(ordinary).includes(basename(file)));
});

// 只执行真实respond函数的内存写入适配；合成返回对象不接收Host或业务上下文。
function serialized(output) {
  const respond=source.statements.find(node=>ts.isFunctionDeclaration(node)&&node.name?.text==='respond');
  assert(respond);
  const writes=[],renames=[];
  const context={Buffer,replied:false,LIMIT:64*1024,
    request:{response_path:'synthetic-response.json',entry_path:'UNRELATED_ENTRY_MUST_NOT_BE_INVENTED.mjs',invocation:{invocation_id:'synthetic'}},
    fault:(code,message)=>Object.assign(new Error(message),{code}),
    writeFileSync:(...args)=>writes.push(args),renameSync:(...args)=>renames.push(args)};
  vm.runInNewContext(declaration.getText(source)+'\n'+respond.getText(source)+'\nglobalThis.serialize=respond;',context);
  context.serialize({status:'ok',output});
  assert.equal(writes.length,1);assert.equal(renames.length,1);
  assert.equal(writes[0][0],'synthetic-response.json.tmp');assert.equal(renames[0][1],'synthetic-response.json');
  const result=JSON.parse(writes[0][1]);assert.equal(result.api_version,1);assert.equal(result.invocation_id,'synthetic');
  return result;
}

test('返回值序列化保留toJSON抛错的真实源帧和阶段，不改变错误信封',()=>{
  const result=serialized({toJSON(){throw new Error('SERIALIZATION_SOURCE_SENTINEL');}});
  assert.equal(result.status,'error');assert.equal(result.error.code,'invalid_output');
  assert(result.error.message.startsWith('动作结果必须可序列化为 JSON。\n返回值序列化：'));
  assert(result.error.message.includes('SERIALIZATION_SOURCE_SENTINEL'));
  assert(result.error.message.includes(basename(file)));
  assert.match(result.error.message,/node-error-diagnostic\.test\.mjs:\d+:\d+/);
  assert.deepEqual(Object.keys(result.error).sort(),['code','message']);
});

test('返回值序列化对循环值保留引擎事实，不伪造作者入口位置',()=>{
  const output={};output.self=output;const result=serialized(output);
  assert.equal(result.error.code,'invalid_output');assert(result.error.message.includes('返回值序列化：'));
  assert(result.error.message.includes('circular'));assert(!result.error.message.includes('UNRELATED_ENTRY_MUST_NOT_BE_INVENTED'));
});

test('返回值序列化长异常仍保留尾部源帧与原消息和协议预算',()=>{
  const result=serialized({toJSON(){throw new Error('字😀'.repeat(20000));}}),text=result.error.message;
  assert.equal(result.error.code,'invalid_output');assert(text.length<=4096);
  assert(text.includes('返回值序列化：'));assert(text.includes('诊断已截断'));assert(text.includes(basename(file)));
  assert.equal(text,Buffer.from(text,'utf8').toString('utf8'));assert(Buffer.byteLength(JSON.stringify(result),'utf8')<64*1024);
});

test('返回值序列化超限保留原output_limit代码和精确消息',()=>{
  const result=serialized('x'.repeat(70*1024));
  assert.equal(result.status,'error');assert.equal(result.error.code,'output_limit');
  assert.equal(result.error.message,'动作结果超过 64 KiB。');assert.deepEqual(Object.keys(result.error).sort(),['code','message']);
});

test('返回值序列化结构化原因不读取宿主栈且预算边界保持合法Unicode',()=>{
  let stackReads=0;
  const cause=Object.assign(new Error('😀'.repeat(10000)),{code:'synthetic_fault',method:'synthetic.method',effectState:'unknown'});
  Object.defineProperty(cause,'stack',{get(){stackReads++;return 'HOST STACK MUST NOT LEAK';}});
  const result=serialized({toJSON(){throw cause;}}),text=result.error.message;
  assert.equal(result.error.code,'invalid_output');assert(text.includes('返回值序列化：'));
  assert.equal(stackReads,0);assert(!text.includes('HOST STACK MUST NOT LEAK'));
  assert(text.length<=4096);assert.equal(text,Buffer.from(text,'utf8').toString('utf8'));
});

test('返回值序列化抛出null或undefined仍交付invalid_output而不伪造源码位置',()=>{
  for(const value of [null,undefined]) {
    const result=serialized({toJSON(){throw value;}});
    assert.equal(result.status,'error');assert.equal(result.error.code,'invalid_output');
    assert.equal(result.error.message,'动作结果必须可序列化为 JSON。\n返回值序列化：'+String(value));
    assert(!result.error.message.includes('UNRELATED_ENTRY_MUST_NOT_BE_INVENTED'));
  }
});
