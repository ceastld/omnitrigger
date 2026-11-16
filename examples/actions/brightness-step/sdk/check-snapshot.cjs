// 开发检查只读取本包声明，source 字段是来源标签，不作为读取路径。
const {readFileSync}=require('node:fs');
const {resolve,join}=require('node:path');
const {createHash}=require('node:crypto');
try {
  const root=resolve(process.argv[2]??join(__dirname,'..'));
  const snapshot=JSON.parse(readFileSync(join(root,'sdk/snapshot.json'),'utf8'));
  const names=['api.d.ts','script-types.d.ts','clipboard-history-api.d.ts'];
  if(snapshot.schema_version!==1||!snapshot.files||JSON.stringify(Object.keys(snapshot.files).sort())!==JSON.stringify([...names].sort()))throw Object.assign(new Error('SDK 快照清单无效。'),{code:'sdk_snapshot_invalid'});
  const mismatches=[];
  for(const name of names){
    const expected=snapshot.files[name]?.sha256;
    if(typeof expected!=='string'||!/^[a-f0-9]{64}$/.test(expected))throw Object.assign(new Error('SDK 快照指纹无效。'),{code:'sdk_snapshot_invalid'});
    const actual=createHash('sha256').update(readFileSync(join(root,'sdk',name))).digest('hex');
    if(actual!==expected)mismatches.push({file:name,expected,actual});
  }
  console.log(JSON.stringify({passed:mismatches.length===0,files:names.length,mismatches,authorCodeExecuted:false,runtimeVerified:false}));
  if(mismatches.length)process.exitCode=1;
} catch(error){
  console.error(JSON.stringify({passed:false,error:{code:error.code??'sdk_snapshot_check_failed',message:error.message},authorCodeExecuted:false}));
  process.exitCode=1;
}
