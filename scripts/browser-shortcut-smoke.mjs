// 仅在隔离 Chrome 内核验快捷键业务结果；本模块不创建或选择用户浏览器连接。
import assert from 'node:assert/strict';
import {randomUUID} from 'node:crypto';
import {mkdir, readFile, writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {spawn} from 'node:child_process';
import {setTimeout as delay} from 'node:timers/promises';

export async function verifyDesktopShortcuts({context, cdp, call, base, scratch, app, root, fixture}) {
  const cases = JSON.parse(await readFile(fixture,'utf8'));
  const operations = new Map([['163,161,9','previous'],['163,9','next'],['163,87','close']]);
  assert.equal(cases.length,3);
  assert.equal(new Set(cases.map(item=>item.action.keys.join(','))).size,3);
  assert(cases.every(item=>operations.has(item.action.keys.join(','))), '仅允许三种已核对的标签快捷键');
  const processInfo = await cdp.send('SystemInfo.getProcessInfo');
  const processId = processInfo.processInfo.find(item=>item.type==='browser')?.id;
  assert(Number.isInteger(processId));
  const marker = `/shortcut-${randomUUID()}/`;
  const directory = join(scratch,'actions/shortcut-probe');
  await mkdir(directory,{recursive:true});
  await writeFile(join(directory,'action.json'),JSON.stringify({schema_version:1,id:'shortcut-probe',name:'隔离快捷键验收',version:'1.0.0',
    actions:[{id:'send',title:'隔离快捷键验收',version:1,runtime:'node',entry:'main.mjs'}]}));
  await writeFile(join(directory,'main.mjs'),`import send from ${JSON.stringify(pathToFileURL(join(root,'examples/actions/shortcut/main.mjs')).href)};
export default async ctx => {
  const target = await ctx.desktop.target();
  if (!target.isForeground || target.processId !== ctx.input.processId || target.processName.toLowerCase() !== 'chrome.exe' || !target.title.startsWith(ctx.input.titlePrefix)) {
    throw new Error('前台不是本次隔离测试窗口，未发送输入');
  }
  return await send({...ctx,input:ctx.input.action});
};`);
  const observed=[];
  for (const item of cases) {
    const window = await call('windows.create',[{url:['a','b','c'].map(name=>base+marker+name),focused:true}]);
    try {
      const ids = (await call('tabs.query',[{windowId:window.id}])).sort((a,b)=>a.index-b.index).map(tab=>tab.id);
      assert.equal(ids.length,3);
      for (const name of ['a','b','c']) {
        let page;
        for (let attempt=0;attempt<100;attempt++) {
          page=context.pages().find(page=>page.url()===base+marker+name);
          if(page) break;
          assert(attempt<99,'尚未观察到本次隔离页面');await delay(50);
        }
        await page.waitForLoadState();
        assert.equal(await page.title(),'OmniTrigger '+marker+name);
      }
      await call('tabs.update',[ids[1],{active:true}]);
      await call('windows.update',[window.id,{focused:true}]);
      const page = context.pages().find(page=>page.url()===base+marker+'b');
      assert(page); await page.bringToFront();
      const callPath=join(scratch,'shortcut-call.json');
      await writeFile(callPath,JSON.stringify({package_id:'shortcut-probe',action_id:'send',action_version:1,
        input:{processId,titlePrefix:'OmniTrigger '+marker,action:item.action}}));
      const child=spawn(app,['--config',join(scratch,'bindings.json'),'--run-action',callPath],{windowsHide:true,stdio:['ignore','pipe','pipe']});
      let stdout='',stderr='';child.stdout.on('data',data=>{stdout+=data;});child.stderr.on('data',data=>{stderr+=data;});
      const timer=setTimeout(()=>child.kill(),20000);
      try {await new Promise((resolve,reject)=>{child.once('error',reject);child.once('exit',code=>code===0?resolve():reject(Error(`快捷键调用失败 ${code}: ${stderr} ${stdout}`)));});}
      finally {clearTimeout(timer);}
      const result=JSON.parse(stdout); assert.equal(result.status,'succeeded'); assert.deepEqual(result.output,{submitted:true});
      const operation=operations.get(item.action.keys.join(','));
      let after;
      for(let attempt=0;attempt<100;attempt++) {
        after=(await call('tabs.query',[{windowId:window.id}])).sort((a,b)=>a.index-b.index);
        const active=after.find(tab=>tab.active)?.id;
        const done=operation==='close' ? after.length===2 && !after.some(tab=>tab.id===ids[1]) && [ids[0],ids[2]].includes(active)
          : after.length===3 && active===ids[operation==='previous'?0:2];
        if(done) break;
        assert(attempt<99,'系统接纳输入但浏览器后置条件未完成');await delay(50);
      }
      observed.push({case:item.name,operation,remainingTabs:after.length,activeIndex:after.findIndex(tab=>tab.active),verified:true});
    } finally {await call('windows.remove',[window.id]);}
  }
  return observed;
}
