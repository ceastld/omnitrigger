import path from 'node:path';

const MAX_STEPS = 5;
const MAX_ENTRIES = 1024;
const PAGE_SIZE = 24;
/** @param {string} code @param {'not_started'|'unknown'} [effectState] */
function fault(code, effectState='not_started') {
  return Object.assign(new Error('文件夹预览未完成，请检查结构化错误。'), {code,effectState});
}
/** @param {unknown} value @returns {value is string} */
function validText(value) {
  if (typeof value !== 'string' || value.includes('\0')) return false;
  for (let i=0;i<value.length;i++) {
    const unit=value.charCodeAt(i);
    if (unit>=0xd800&&unit<=0xdbff) {
      const next=value.charCodeAt(++i);
      if (!(next>=0xdc00&&next<=0xdfff)) return false;
    } else if (unit>=0xdc00&&unit<=0xdfff) return false;
  }
  return true;
}
/** @param {unknown} value */
export function localPath(value) {
  if (!validText(value)||!value||value.length>1024||!/^[A-Za-z]:\\/.test(value)
      ||value.slice(2).includes(':')||/[\x00-\x1f<>"|?*]/.test(value)
      ||value.split(/[\\/]/).some(part=>/^(OneDrive.*|Dropbox|iCloud.*)$/i.test(part))) throw fault('invalid_input');
  const normalized=path.win32.normalize(value);
  return normalized.length>3?normalized.replace(/\\+$/,''):normalized;
}
/** @param {string} root @param {string} candidate */
function within(root,candidate) {
  const relative=path.win32.relative(root,candidate);
  return relative===''||(!relative.startsWith('..\\')&&relative!=='..'&&!path.win32.isAbsolute(relative));
}
/** @param {string} value @param {number} max */
function short(value,max) {
  // 仅缩短菜单显示，打开/校验始终使用完整原路径。
  const text=value.replace(/[\r\n\t]/g,' ');
  if (text.length<=max) return text;
  let end=max-1;
  if (end>0&&/[\uD800-\uDBFF]/.test(text[end-1])) end--;
  return text.slice(0,end)+'…';
}
/** @param {import('./sdk/api').LocalDirectoryResult} value @param {string} root @param {string} current
 * @returns {import('./sdk/api').LocalDirectoryResult} */
export function parseListing(value,root,current) {
  if (!value||Object.keys(value).sort().join(',')!=='complete,entries'||value.complete!==true
      ||!Array.isArray(value.entries)||value.entries.length>MAX_ENTRIES
      ||Buffer.byteLength(JSON.stringify(value),'utf8')>65536) throw fault('invalid_response');
  const unique=new Set();
  for (const entry of value.entries) {
    if (!entry||Object.keys(entry).sort().join(',')!=='kind,name,path,reason'
        ||!['file','directory','other','unavailable'].includes(entry.kind)
        ||!validText(entry.name)||!entry.name||entry.name==='.'||entry.name==='..'||/[\\/]/.test(entry.name)
        ||!validText(entry.path)||entry.path.length>1024
        ||entry.path!==current+(current.endsWith('\\')?'':'\\')+entry.name
        ||!within(root,entry.path)||unique.has(entry.path.toLowerCase())
        ||(entry.kind==='unavailable'?!['reparse_point','cloud_placeholder','cloud_path','unsupported_name'].includes(entry.reason??''):entry.reason!==null)) throw fault('invalid_response');
    if (entry.kind==='file'||entry.kind==='directory') {
      try { if (localPath(entry.path)!==entry.path) throw fault('invalid_response'); }
      catch { throw fault('invalid_response'); }
    }
    unique.add(entry.path.toLowerCase());
  }
  // 仍以文件夹优先；各组内沿用 API 的名称排序。不可用项保留为明确可见的菜单项。
  const entries=[...value.entries].sort((a,b)=>Number(b.kind==='directory')-Number(a.kind==='directory'));
  return {entries,complete:true};
}
/** @param {import('./sdk/api').ActionContext} ctx @param {string} target @param {'file'|'directory'} kind */
async function requiredKind(ctx,target,kind) {
  const result=await ctx.fs.stat(target);
  if (result.kind!==kind) throw fault('path_changed');
}
/** @param {import('./sdk/api').ActionContext} ctx */
function checkCancelled(ctx) {
  if (ctx.signal?.aborted) throw fault('cancelled');
}
/** @param {import('./sdk/api').ChooseResult} answer @returns {answer is Extract<import('./sdk/api').ChooseResult,{status:'selected'}>} */
function selection(answer) {
  if (answer?.status==='cancelled') return false;
  if (answer?.status!=='selected'||typeof answer.id!=='string') throw fault('invalid_response');
  return true;
}
/** @param {import('./sdk/api').ActionContext} ctx @param {string} target */
async function submitOpen(ctx,target) {
  checkCancelled(ctx);
  const result=await ctx.open(target);
  if (result?.submitted!==true) throw fault('invalid_response','unknown');
}
/** Node 异步入口；SDK 类型仅用于开发检查。
 * @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx) {
  const input=ctx.input ?? {};
  if (!input||typeof input!=='object'||Array.isArray(input)||Object.keys(input).some(key=>key!=='roots')
      ||input.roots!==undefined&&(!Array.isArray(input.roots)||input.roots.length>4)) throw fault('invalid_input');
  /** @type {readonly unknown[]} */
  let requested=input.roots ?? [];
  if (!requested||requested.length===0) {
    checkCancelled(ctx);
    if (typeof ctx.desktop?.explorerContext!=='function') throw fault('explorer_context_unavailable');
    const context=await ctx.desktop.explorerContext();
    checkCancelled(ctx);
    requested=context.selectedPaths;
    if (!Array.isArray(requested)||requested.length>4) throw fault('selection_too_large');
    if (requested.length===0) throw fault('selection_empty');
  }
  const roots=requested.map(localPath);
  if (new Set(roots.map(value=>value.toLowerCase())).size!==roots.length) throw fault('invalid_input');
  if (typeof ctx.fs?.stat!=='function'||typeof ctx.ui?.choose!=='function'
      ||typeof ctx.open!=='function') throw fault('runtime_unavailable');
  const rootKinds=[];
  for (const root of roots) {
    checkCancelled(ctx);
    const value=await ctx.fs.stat(root);
    if (!['file','directory'].includes(value.kind)) throw fault('path_unavailable');
    rootKinds.push(value.kind);
  }
  checkCancelled(ctx);
  const first=await ctx.ui.choose({title:'文件菜单',placeholder:'选择文件或文件夹',timeoutMs:30000,
    items:roots.map((root,index)=>({id:'root:'+index,label:short(path.win32.basename(root)||root,96),description:short(root,180)}))});
  if (!selection(first)) return {status:'cancelled'};
  const rootIndex=roots.findIndex((_,index)=>first.id==='root:'+index);
  if (rootIndex<0) throw fault('invalid_response');
  let root=roots[rootIndex],current=root;
  if (rootKinds[rootIndex]==='file') {
    checkCancelled(ctx); await requiredKind(ctx,root,'file'); checkCancelled(ctx);
    await submitOpen(ctx,root); return {status:'open_submitted',kind:'file'};
  }
  if (typeof ctx.fs.readDirectory!=='function') throw fault('directory_enumeration_unavailable');
  /** @type {import('./sdk/api').LocalDirectoryResult|null} */
  let listing=null;
  let page=0;
  for (let step=0;step<MAX_STEPS;step++) {
    checkCancelled(ctx);
    if (listing===null) {
      await requiredKind(ctx,current,'directory');
      const result=await ctx.fs.readDirectory(current,{maxEntries:MAX_ENTRIES});
      listing=parseListing(result,root,current);
      page=0;
    }
    checkCancelled(ctx);
    const start=page*PAGE_SIZE;
    const visible=listing.entries.slice(start,start+PAGE_SIZE);
    const pages=Math.max(1,Math.ceil(listing.entries.length/PAGE_SIZE));
    /** @type {import('./sdk/api').ChoiceItem[]} */
    const items=[{id:'open-current',label:'打开此文件夹',description:short(current,180)}];
    if (current.toLowerCase()!==root.toLowerCase()) items.push({id:'parent',label:'返回上级'});
    visible.forEach((entry,index)=>items.push({id:'entry:'+(start+index),label:short((entry.kind==='directory'?'[文件夹] ':entry.kind==='file'?'':'[不可用] ')+entry.name,96),description:short(entry.path,180)}));
    // 翻页只重用本目录已读快照，仍消耗一轮；不会重置 SDK 或本次预览预算。
    if (page>0) items.push({id:'previous-page',label:'上一页',description:'翻页计入本次5轮预览上限'});
    if (page+1<pages) items.push({id:'next-page',label:'下一页',description:'翻页计入本次5轮预览上限'});
    const title=pages===1?'文件夹预览':`文件夹预览（第${page+1}/${pages}页，共${listing.entries.length}项）`;
    const answer=await ctx.ui.choose({title,items,timeoutMs:30000});
    if (!selection(answer)) return {status:'cancelled'};
    checkCancelled(ctx);
    if (answer.id==='previous-page'&&page>0) {page--;continue;}
    if (answer.id==='next-page'&&page+1<pages) {page++;continue;}
    if (answer.id==='open-current') {
      await requiredKind(ctx,current,'directory'); checkCancelled(ctx); await submitOpen(ctx,current);
      return {status:'open_submitted',kind:'directory'};
    }
    if (answer.id==='parent'&&current.toLowerCase()!==root.toLowerCase()) {
      current=path.win32.dirname(current); listing=null; page=0; continue;
    }
    const index=visible.findIndex((_,index)=>answer.id==='entry:'+(start+index));
    if (index<0) throw fault('invalid_response');
    const selected=visible[index];
    if (selected.kind!=='file'&&selected.kind!=='directory') throw fault('path_unavailable');
    await requiredKind(ctx,selected.path,selected.kind);
    checkCancelled(ctx);
    if (selected.kind==='file') {
      await submitOpen(ctx,selected.path); return {status:'open_submitted',kind:'file'};
    }
    current=selected.path; listing=null; page=0;
  }
  return {status:'navigation_limit',openSubmitted:false};
}
