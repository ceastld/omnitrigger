// 在临时工程中，经应用 CLI 与真实脚本宿主核验本地元数据；不启动界面或 Hook。
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdirSync, mkdtempSync, readFileSync, realpathSync, symlinkSync, writeFileSync} from 'node:fs';
import {dirname, isAbsolute, join, relative, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

/** 可复用到实际安装包验收；entryPath、call 和 root 都必须属于调用者新建的验收工程。 */
export function verifyFsStat({root, entryPath, call, invoke, node = false}) {
  const within = (parent, child) => {
    const name = relative(parent, child);
    return name !== '' && name !== '..' && !name.startsWith('..\\') && !name.startsWith('../') && !isAbsolute(name);
  };
  assert(within(resolve(root),resolve(entryPath)), '验收入口必须位于本轮临时工程内');
  assert(within(realpathSync(root),realpathSync(entryPath)), '验收入口不能通过链接越出临时工程');
  const fixtures = join(root, 'stat-fixtures');
  mkdirSync(fixtures);
  const file = join(fixtures, '本地文件 🦀.txt');
  const directory = join(fixtures, 'folder');
  const junction = join(fixtures, 'junction');
  writeFileSync(file, '只读元数据验收，不应由 stat 打开此内容。');
  mkdirSync(directory);
  symlinkSync(directory, junction, 'junction');
  const cases = [
    {name:'file', path:file, value:{kind:'file'}},
    {name:'directory', path:directory, value:{kind:'directory'}},
    {name:'missing', path:join(fixtures, 'absent.txt'), value:{kind:'missing'}},
    {name:'missing-parent', path:join(fixtures, 'absent', 'child'), value:{kind:'missing'}},
    {name:'forward-slashes', path:file.replaceAll('\\', '/'), value:{kind:'file'}},
    {name:'cloud-name', path:join(fixtures, 'OneDrive-synthetic', 'absent'), code:'path_unavailable'},
    {name:'junction', path:junction, code:'path_unavailable'},
    {name:'junction-child', path:join(junction, 'absent'), code:'path_unavailable'},
    {name:'relative', path:'relative.txt', code:'invalid_input'},
    {name:'unc', path:'\\\\server\\share\\file.txt', code:'invalid_input'},
    {name:'alternate-stream', path:file + ':stream', code:'invalid_input'},
  ];
  const source = `${node ? 'export default async function main' : 'export function main'}(ctx) {
    const result = [];
    for (const item of ctx.input.cases) {
      try { result.push({name:item.name,value:${node ? 'await ' : ''}ctx.fs.stat(item.path)}); }
      catch (error) { result.push({name:item.name,error:{code:error.code,method:error.method,effectState:error.effectState}}); }
    }
    return result;
  }`;
  const original = readFileSync(entryPath);
  try {
    writeFileSync(entryPath, source);
    const callPath = join(root, 'fs-stat-call.json');
    writeFileSync(callPath, JSON.stringify({...call, input:{cases:cases.map(({name,path})=>({name,path}))}}));
    const response = invoke(callPath);
    assert.equal(response.status, 'succeeded', JSON.stringify(response));
    const expected = cases.map(item => item.code
      ? {name:item.name,error:{code:item.code,method:'fs.stat',effectState:'not_started'}}
      : {name:item.name,value:item.value});
    assert.deepEqual(response.output, expected);
    let recipeCases = 0;
    if (!node) {
      const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
      const recipe = readFileSync(join(repo,'examples/actions/common-search/open-or-search.js'),'utf8');
      // 执行实际示例源码及真实 fs API；仅替换 open，避免验收打开浏览器或文件程序。
      writeFileSync(entryPath, `function runRecipe(ctx) {\n${recipe}\nreturn main(ctx);\n}
        export function main(ctx) {
          const results=[];
          for(const item of ctx.input.cases) {
            const opened=[];
            const local={...ctx,input:{source:'text',text:item.path,urlTemplate:'https://example.invalid/search?q=%s'},open:target=>{opened.push(target);}};
            try { results.push({name:item.name,output:runRecipe(local),opened}); }
            catch(error) { results.push({name:item.name,error:{code:error.code,method:error.method,effectState:error.effectState},opened}); }
          }
          return results;
        }`);
      const selected = cases.filter(item=>['file','directory','missing','cloud-name','junction-child'].includes(item.name));
      writeFileSync(callPath,JSON.stringify({...call,input:{cases:selected.map(({name,path})=>({name,path}))}}));
      const result = invoke(callPath);
      assert.equal(result.status,'succeeded',JSON.stringify(result));
      const expectedRecipe = selected.map(item=>{
        if(item.code) return {name:item.name,error:{code:item.code,method:'fs.stat',effectState:'not_started'},opened:[]};
        if(item.value.kind==='file') return {name:item.name,output:{submitted:true,source:'text',operation:'open-file'},opened:[item.path]};
        const points=Array.from(item.path);
        return {name:item.name,output:{submitted:true,source:'text',operation:'search',queryCodePoints:Math.min(points.length,128),truncated:points.length>128,pathKind:item.value.kind},opened:['https://example.invalid/search?q='+encodeURIComponent(points.slice(0,128).join(''))]};
      });
      assert.deepEqual(result.output,expectedRecipe);
      recipeCases=selected.length;
    }
    return {passed:true,cases:cases.length,recipeCases,runtime:node ? 'node' : 'script',noUi:true,personalBindingsModified:false};
  } finally {
    writeFileSync(entryPath, original);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  assert.equal(process.platform, 'win32', '此检查使用 Windows 本地路径和目录联接。');
  const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
  const app = resolve(process.argv[2] ?? 'target/debug/omnitrigger.exe');
  const scriptHosts = process.argv.slice(3).map(p=>resolve(p));
  mkdirSync(join(repo,'.tmp'), {recursive:true});
  const root = mkdtempSync(join(repo,'.tmp/fs-stat-check-'));
  const modes = [{name:'node',node:true}, ...scriptHosts.map((host,index)=>({name:`script-${index+1}`,node:false,host}))];
  const checks = [];
  for (const mode of modes) {
    const current = join(root, mode.name);
    const packageRoot = join(current, 'actions', 'stat-check');
    mkdirSync(packageRoot, {recursive:true});
    const config = join(current, 'bindings.json');
    writeFileSync(config, '[]');
    const entryPath = join(packageRoot, 'main.mjs');
    writeFileSync(entryPath, mode.node ? 'export default () => null;' : 'export function main() { return null; }');
    writeFileSync(join(packageRoot, 'action.json'), JSON.stringify({schema_version:1,id:'stat-check',name:'本地元数据验收',version:'1.0.0',actions:[{id:'stat',title:'元数据验收',version:1,runtime:mode.node?'node':'script',entry:'main.mjs'}]}));
    const env = {...process.env,OMNITRIGGER_NODE:process.execPath};
    if (mode.host) env.OMNITRIGGER_SCRIPT_HOST = mode.host;
    else delete env.OMNITRIGGER_SCRIPT_HOST;
    const result = verifyFsStat({root:current,entryPath,node:mode.node,call:{package_id:'stat-check',action_id:'stat',action_version:1,input:{}},invoke:callPath=>{
      const child = spawnSync(app, ['--config', config, '--run-action', callPath], {encoding:'utf8',windowsHide:true,timeout:30000,env,maxBuffer:1024*1024});
      assert.ifError(child.error);
      assert.equal(child.status,0,child.stderr || child.stdout);
      return JSON.parse(child.stdout);
    }});
    assert.equal(readFileSync(config,'utf8'), '[]');
    checks.push({...result,mode:mode.name});
  }
  const result = {passed:true,root,checks};
  writeFileSync(join(root,'result.json'),JSON.stringify(result,null,2));
  console.log(JSON.stringify(result));
}
