// 对显式提供的源码快照检查轻量宿主模块路径与静态加载预算；没有文件或进程 API。
import ts from 'typescript';
import {Buffer} from 'node:buffer';
import {javascriptSyntaxDiagnostics} from './javascript-syntax.mjs';

const FILE_LIMIT=128*1024, TOTAL_LIMIT=1024*1024, MODULE_LIMIT=64;
const SNAPSHOT_FILES=256, SNAPSHOT_BYTES=8*1024*1024;
const messages={
  snapshot_invalid:'需要明确入口和不重复的包内源码快照。',
  snapshot_limit:'源码快照超过 256 个文件或合计 8 MiB。',
  snapshot_utf8:'源码快照包含无法编码为有效 UTF-8 的代理字符。',
  module_entry:'轻量模块入口必须是包内 .mjs 路径，且不能以点开头。',
  module_specifier:'导入必须是 ./ 或 ../ 开头并显式包含 .js 或 .mjs 的包内路径。',
  module_import_attributes:'轻量模块兼容检查不接受 import/export 属性块；请移除 with/assert。',
  module_path:'模块引用超出动作包。',
  snapshot_module_missing:'所需模块未包含在显式源码快照中。',
  module_file_limit:'模块单文件超过 128 KiB。',
  module_total_limit:'静态加载模块超过 64 个或源码合计超过 1 MiB。',
  dynamic_import_unresolved:'动态导入需要运行时参数，静态图未覆盖其目标和预算。',
  dynamic_import_deferred:'字面动态导入已检查路径，但未计入静态加载图和预算。',
  module_syntax_invalid:'源码存在语法错误，静态模块图未完整分析。',
  module_typescript_syntax:'JavaScript 模块包含 TypeScript 专有语法；请改用 JSDoc 类型注释，不会自动转译。',
  module_analysis_limit:'源码解析或遍历超过分析器可处理的复杂度。',
};
function scalar(text) {
  for(let i=0;i<text.length;i++) {
    const c=text.charCodeAt(i);
    if(c>=0xd800&&c<=0xdbff) {const next=text.charCodeAt(++i);if(!(next>=0xdc00&&next<=0xdfff))return false;}
    else if(c>=0xdc00&&c<=0xdfff)return false;
  }
  return true;
}
function pathParts(base,specifier,relativeImport) {
  if(typeof specifier!=='string'||!specifier||Buffer.byteLength(specifier)>2048
    ||/[\u0000-\u001f\u007f-\u009f\\:?#%]/u.test(specifier)||specifier.startsWith('/')
    ||relativeImport&&!specifier.startsWith('./')&&!specifier.startsWith('../'))return {code:'module_specifier'};
  const parts=[...base];
  for(const part of specifier.split('/')) {
    if(part==='.')continue;
    if(part==='..') {if(!parts.length)return {code:'module_path'};parts.pop();continue;}
    const stem=part.split('.')[0].replace(/ +$/u,'').toUpperCase();
    if(!part||/[. ]$/u.test(part)||/[<>"|*]/u.test(part)
      ||['CON','PRN','AUX','NUL','CONIN$','CONOUT$','CLOCK$'].includes(stem)||/^(?:COM|LPT)[1-9¹²³]$/u.test(stem))return {code:'module_specifier'};
    parts.push(part);
  }
  const path=parts.join('/');
  if(!/\.(?:js|mjs)$/u.test(path))return {code:'module_specifier'};
  return {path};
}

/** 类型检查之外的静态检查；sources 是 [{path,text}]，不会按路径读取磁盘。 */
export function analyzeScriptModules(snapshot) {
  const diagnostics=[],edges=[],modules=[];
  let totalBytes=0,partial=false;
  const add=(code,file,node,source,severity='error')=>{
    const point=source&&node!==undefined&&node!==null
      ?source.getLineAndCharacterOfPosition(typeof node==='number'?node:node.getStart(source)):undefined;
    diagnostics.push({code,severity,message:messages[code],...(file?{file}:{}),
      ...(point?{line:point.line+1,column:point.character+1}:{})});
  };
  const result=()=>({passed:!diagnostics.some(d=>d.severity==='error'),staticGraphComplete:!partial&&!diagnostics.some(d=>d.severity==='error'),
    modules,edges,totalBytes,diagnostics,runtimeExecuted:false,filesRead:false,filesModified:false,
    scope:'provided_source_snapshot_only',runtimeVerified:false});
  if(!snapshot||typeof snapshot!=='object'||Array.isArray(snapshot)||typeof snapshot.entry!=='string'||!Array.isArray(snapshot.sources)) {
    add('snapshot_invalid');return result();
  }
  if(snapshot.sources.length>SNAPSHOT_FILES) {add('snapshot_limit');return result();}
  const sources=new Map();let snapshotBytes=0;
  for(const item of snapshot.sources) {
    if(!item||typeof item!=='object'||typeof item.path!=='string'||typeof item.text!=='string') {add('snapshot_invalid');return result();}
    const checked=pathParts([],item.path,false);
    if(checked.path!==item.path||sources.has(item.path)) {add('snapshot_invalid');return result();}
    if(!scalar(item.text)||!scalar(item.path)) {add('snapshot_utf8');return result();}
    snapshotBytes+=Buffer.byteLength(item.text);
    if(snapshotBytes>SNAPSHOT_BYTES) {add('snapshot_limit');return result();}
    sources.set(item.path,item.text);
  }
  const entry=pathParts([],snapshot.entry,false);
  if(!entry.path||entry.path!==snapshot.entry||!snapshot.entry.endsWith('.mjs')||snapshot.entry.startsWith('.')) {
    add('module_entry');return result();
  }
  const visited=new Set(),queue=[{file:entry.path}];
  for(let index=0;index<queue.length;index++) {
    const {file,from,node:importNode,source:importSource}=queue[index];
    if(visited.has(file))continue;
    visited.add(file);
    const text=sources.get(file);
    if(text===undefined) {add('snapshot_module_missing',from??file,importNode,importSource);continue;}
    const bytes=Buffer.byteLength(text);
    if(bytes>FILE_LIMIT) {add('module_file_limit',file);continue;}
    if(modules.length===MODULE_LIMIT||totalBytes+bytes>TOTAL_LIMIT) {add('module_total_limit',file);break;}
    modules.push(file);totalBytes+=bytes;
    let source;
    try { source=ts.createSourceFile(file,text,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS); }
    catch { add('module_analysis_limit',file);continue; }
    if(source.parseDiagnostics.length) {
      add('module_syntax_invalid',file);continue;
    }
    try {
      const syntax=javascriptSyntaxDiagnostics(source);
      if(syntax.length) {
        for(const diagnostic of syntax)add('module_typescript_syntax',file,diagnostic.start,source);
        continue;
      }
    } catch {add('module_analysis_limit',file);continue;}
    const connect=(node,specifier,dynamic)=>{
      const resolved=pathParts(file.split('/').slice(0,-1),specifier,true);
      if(resolved.code) {add(resolved.code,file,node,source);return;}
      edges.push({from:file,to:resolved.path,kind:dynamic?'dynamic_literal':'static'});
      if(dynamic) {
        partial=true;add('dynamic_import_deferred',file,node,source,'warning');
        if(!sources.has(resolved.path))add('snapshot_module_missing',file,node,source,'warning');
      } else queue.push({file:resolved.path,from:file,node,source});
    };
    const visit=node=>{
      if((ts.isImportDeclaration(node)||ts.isExportDeclaration(node))&&node.moduleSpecifier&&ts.isStringLiteralLike(node.moduleSpecifier)) {
        if(node.attributes) add('module_import_attributes',file,node.attributes,source);
        else connect(node.moduleSpecifier,node.moduleSpecifier.text,false);
      } else if(ts.isCallExpression(node)&&node.expression.kind===ts.SyntaxKind.ImportKeyword) {
        partial=true;
        if(node.arguments.length===1&&ts.isStringLiteralLike(node.arguments[0]))connect(node.arguments[0],node.arguments[0].text,true);
        else add('dynamic_import_unresolved',file,node,source,'warning');
      }
      ts.forEachChild(node,visit);
    };
    try { visit(source); }
    catch { add('module_analysis_limit',file); }
  }
  return result();
}
