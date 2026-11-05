// 只检查 ESM 文本中的 main 具名导出声明；不解析依赖、不执行初始化器。
import ts from 'typescript';
import {Buffer} from 'node:buffer';

export function analyzeScriptEntry(input={}) {
  const valid=input!==null&&typeof input==='object'&&!Array.isArray(input);
  const {file='main.mjs',text}=valid?input:{};
  const diagnostics=[];
  const result=mainExport=>({passed:!diagnostics.some(d=>d.severity==='error'),mainExport,diagnostics,
    declarationAnalysisOnly:true,runtimeVerified:false,runtimeExecuted:false,filesRead:false,filesModified:false});
  const add=(code,severity,message,node,source)=>{
    const point=node&&source?source.getLineAndCharacterOfPosition(node.getStart(source)):undefined;
    diagnostics.push({code,severity,message,file,...(point?{line:point.line+1,column:point.character+1}:{})});
  };
  if(!valid||typeof file!=='string'||file.length>2048||!file.endsWith('.mjs')||typeof text!=='string'||Buffer.byteLength(text)>128*1024) {
    add('entry_snapshot_invalid','error','需要不超过 128 KiB 的明确 .mjs 入口文本。');
    return result('unresolved');
  }
  let source;
  try {source=ts.createSourceFile(file,text,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);}
  catch {add('entry_analysis_limit','error','源码复杂度超过入口分析器可处理范围。');return result('unresolved');}
  if(source.parseDiagnostics.length) {
    add('entry_syntax_invalid','error','入口源码有语法错误，未确定具名导出。');
    return result('unresolved');
  }
  const locals=new Map(),exported=[],stars=[];
  const has=(node,kind)=>node.modifiers?.some(m=>m.kind===kind);
  const callable=value=>value&&(ts.isArrowFunction(value)||ts.isFunctionExpression(value))?'callable':
    value&&(ts.isStringLiteralLike(value)||ts.isNumericLiteral(value)||ts.isClassExpression(value)
      ||ts.isObjectLiteralExpression(value)||ts.isArrayLiteralExpression(value)
      ||[ts.SyntaxKind.TrueKeyword,ts.SyntaxKind.FalseKeyword,ts.SyntaxKind.NullKeyword].includes(value.kind))?'noncallable':'unresolved';
  const bind=(name,kind)=>{
    if(ts.isIdentifier(name))locals.set(name.text,kind);
    else for(const element of name.elements)if(ts.isBindingElement(element))bind(element.name,'unresolved');
  };
  for(const node of source.statements) {
    const direct=has(node,ts.SyntaxKind.ExportKeyword)&&!has(node,ts.SyntaxKind.DefaultKeyword);
    if(ts.isFunctionDeclaration(node)&&node.name) {
      locals.set(node.name.text,'callable');
      if(direct&&node.name.text==='main')exported.push({local:'main',node});
    } else if(ts.isClassDeclaration(node)&&node.name) {
      locals.set(node.name.text,'noncallable');
      if(direct&&node.name.text==='main')exported.push({local:'main',node});
    } else if(ts.isVariableStatement(node)) {
      const previous=locals.get('main');
      for(const declaration of node.declarationList.declarations) {
        bind(declaration.name,callable(declaration.initializer));
        if(direct&&ts.isIdentifier(declaration.name)&&declaration.name.text==='main')exported.push({local:'main',node:declaration});
        else if(direct&&!ts.isIdentifier(declaration.name)&&locals.has('main')&&locals.get('main')!==previous)exported.push({local:'main',node:declaration});
      }
    } else if(ts.isExportDeclaration(node)) {
      if(!node.exportClause)stars.push(node);
      else if(ts.isNamedExports(node.exportClause)) {
        for(const item of node.exportClause.elements)if(item.name.text==='main') {
          exported.push({local:node.moduleSpecifier?undefined:(item.propertyName??item.name).text,node:item});
        }
      } else if(ts.isNamespaceExport(node.exportClause)&&node.exportClause.name.text==='main') {
        exported.push({kind:'noncallable',node});
      }
    }
  }
  if(exported.length===0) {
    if(stars.length) {
      add('entry_main_reexport_unresolved','warning','星号再导出的 main 需要解析依赖后才能确定。',stars[0],source);
      return result('unresolved_export');
    }
    add('entry_main_not_exported','error','静态入口没有 main 具名导出；默认导出或仅声明局部 main 不满足该入口契约。');
    return result('missing_named_export');
  }
  const entry=exported[0],kind=entry.kind??locals.get(entry.local)??'unresolved';
  if(kind==='noncallable') {
    add('entry_main_noncallable','error','main 的静态声明是不可直接调用的值或 class。',entry.node,source);
    return result('local_noncallable_declaration');
  }
  if(kind==='unresolved') {
    add('entry_main_value_unresolved','warning','main 来自导入、再导出或运行时表达式，静态检查未确定实际值。',entry.node,source);
    return result('unresolved_export');
  }
  // 声明是函数也不证明初始化后仍为函数；重赋值与任意初始化副作用交给显式运行验收。
  return result('local_callable_declaration');
}
