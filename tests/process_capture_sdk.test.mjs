// 只在 VM 中调用 JSON RecordingHost；不启动程序或真实脚本宿主。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';

const runtime = new URL('../crates/omnitrigger-actions/runtime/',import.meta.url);
const source = ['desktop-api.js','browser-api.js','clipboard-history-api.js','process-capture-api.js','actions-api.js']
  .map(name=>readFileSync(new URL(name,runtime),'utf8')).join('\n')
  + '\n' + readFileSync(new URL('../tools/js-engine-probe/src/sdk.js',import.meta.url),'utf8');
const program='C:\\owned fixture\\program.exe';
function host(value={code:0,stdout:'中😀',stderr:''}, error) {
  const requests=[];
  const scope={ctx:{input:{}},__hostCall:json=>{
    const request=JSON.parse(json);requests.push(request);
    return JSON.stringify(error?{ok:false,error}:{ok:true,value});
  }};
  runInNewContext(source,scope);
  return {api:scope.ctx.process,requests};
}

test('capture 是独立 wire，默认10秒/非零拒绝，不改变run/start',()=>{
  const h=host();
  assert.deepEqual(JSON.parse(JSON.stringify(h.api.capture(program))),{code:0,stdout:'中😀',stderr:''});
  assert.deepEqual(h.requests,[{kind:'capture',program,args:[],timeout_ms:10000,allow_non_zero:false}]);
  const run=host({pid:12,exitCode:0,success:true});
  assert.deepEqual(JSON.parse(JSON.stringify(run.api.run('existing-program'))),{pid:12,exitCode:0,success:true});
  assert.deepEqual(run.requests,[{kind:'run',program:'existing-program',args:[],timeout_ms:1000}]);
  const start=host({pid:13});start.api.start('background-program',['arg']);
  assert.deepEqual(start.requests,[{kind:'start',program:'background-program',args:['arg']}]);
});

test('参数、cwd和等待策略在任何JSON宿主请求前严格拒绝',()=>{
  const h=host();
  for(const badProgram of ['python','C:\\x.cmd','C:\\x.bat','\\\\server\\x.exe','C:\\..\\x.exe','C:\\x\0.exe','C:\\x\ud800.exe']){
    assert.throws(()=>h.api.capture(badProgram),e=>e.code==='invalid_input'&&e.effectState==='not_started');
  }
  for(const options of [null,[],{timeoutMs:0},{timeoutMs:60001},{timeoutMs:'1'},{allowNonZero:1},
    {cwd:null},{cwd:'relative'},{cwd:'C:\\..\\x'},{extra:1},{timeoutMs:undefined}]){
    assert.throws(()=>h.api.capture(program,[],options),e=>e.code==='invalid_input');
  }
  for(const args of [null,'arg',[1],['\0'],['\ud800'],Array(129).fill('x'),Array(1),['x'.repeat(32767)]]){
    assert.throws(()=>h.api.capture(program,args),e=>e.code==='invalid_input');
  }
  assert.deepEqual(h.requests,[]);
});

test('显式cwd、特殊字符参数和非零策略无shell拼接或重试',()=>{
  const h=host({code:7,stdout:'output',stderr:'error'});
  const args=['a b','a"b','tail\\','&echo %PATH%'];
  const result=h.api.capture(program,args,{timeoutMs:60000,allowNonZero:true,cwd:'D:\\owned'});
  assert.equal(result.code,7);
  assert.deepEqual(h.requests,[{kind:'capture',program,args,timeout_ms:60000,allow_non_zero:true,cwd:'D:\\owned'}]);
  const refused=host({code:7,stdout:'private output',stderr:'private error'});
  assert.throws(()=>refused.api.capture(program),e=>e.code==='process_exit_nonzero'&&e.effectState==='unknown'&&!e.message.includes('private'));
  assert.equal(refused.requests.length,1);
});

test('UTF-8字节总量跨两管道共享，NUL是合法UTF-8输出数据',()=>{
  for(const value of [{code:0,stdout:'中'.repeat(21845),stderr:'x'},
    {code:0,stdout:'😀'.repeat(16384),stderr:''},{code:0,stdout:'\0',stderr:''}]){
    assert.equal(host(value).api.capture(program).code,0);
  }
  for(const value of [{code:0,stdout:'a'.repeat(65536),stderr:'x'},
    {code:0,stdout:'中'.repeat(21846),stderr:''},{code:0,stdout:'\ud800',stderr:''},
    {code:0,stdout:'',stderr:'\udfff'},{code:0,stdout:'',stderr:'',extra:1},{code:1.5,stdout:'',stderr:''}]){
    assert.throws(()=>host(value).api.capture(program),e=>e.code==='invalid_response'&&e.effectState==='unknown');
  }
});

test('本机失败只保留固定代码/效果，不回显原始输出或参数',()=>{
  for(const code of ['process_timeout','process_output_too_large','process_invalid_utf8','process_cleanup_failed']){
    const h=host(null,{code,message:'SECRET output '+program,effect_state:'unknown'});
    assert.throws(()=>h.api.capture(program),e=>e.code===code&&e.method==='process.capture'
      &&e.effectState==='unknown'&&!e.message.includes('SECRET')&&!e.stack.includes(program));
    assert.equal(h.requests.length,1);
  }
  const h=host(null,{code:'process_setup_failed',message:'private path'});
  assert.throws(()=>h.api.capture(program),e=>e.code==='process_setup_failed'&&e.effectState==='not_started');
});
