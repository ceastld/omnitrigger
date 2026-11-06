import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {resolve} from 'node:path';

assert.equal(process.argv.length,3,'需要明确的宿主可执行文件路径');
const result=spawnSync(resolve(process.argv[2]),[],{
  input:JSON.stringify({script:'return 42;',input:{}}),
  encoding:'utf8',windowsHide:true,timeout:60000,maxBuffer:128*1024,
});
assert.ifError(result.error);
assert.equal(result.status,0,`宿主启动失败：${result.status} ${result.stderr}`);
assert.deepEqual(JSON.parse(result.stdout),{schema_version:1,ok:true,result:42});
assert.equal(result.stderr,'');
console.log('共享 SDK 启动及同步返回检查通过。');
