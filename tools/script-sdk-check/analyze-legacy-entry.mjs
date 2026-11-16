// 只分析正式旧协议中的 main 绑定和显式动态代码；不求值源码或读取依赖。
import ts from 'typescript';

export function analyzeLegacyEntry(source, program) {
  const diagnostics=[],checker=program.getTypeChecker(),file=source.fileName;
  const add=(code,severity,message,node)=>{
    const point=source.getLineAndCharacterOfPosition(node.getStart(source));
    if(!diagnostics.some(d=>d.code===code&&d.line===point.line+1&&d.column===point.character+1))
      diagnostics.push({code,severity,message,file,line:point.line+1,column:point.character+1});
  };
  const warn=(code,message,node)=>add(code,'warning',message,node);
  const error=(code,message,node)=>add(code,'error',message,node);
  const has=(node,kind)=>node.modifiers?.some(m=>m.kind===kind);
  const fn=node=>node&&(ts.isFunctionDeclaration(node)||ts.isFunctionExpression(node)||ts.isArrowFunction(node));
  const names=node=>ts.isIdentifier(node)?[node]:node.elements?.flatMap(e=>ts.isBindingElement(e)?names(e.name):[])??[];
  const candidates=[];
  for(const node of source.statements) {
    if((ts.isFunctionDeclaration(node)||ts.isClassDeclaration(node))&&node.name?.text==='main')
      candidates.push({name:node.name,callable:fn(node)?node:undefined,value:node});
    if(ts.isVariableStatement(node))for(const d of node.declarationList.declarations)
      for(const name of names(d.name))if(name.text==='main')candidates.push({name,callable:fn(d.initializer)?d.initializer:undefined,value:d.initializer});
  }
  let entry,kind='legacy_unresolved_declaration';
  if(candidates.length>1)error('legacy_entry_main_ambiguous','旧入口需要唯一的 main 声明，不能静默选择重声明。',candidates[1].name);
  else if(candidates.length===1) {
    entry=candidates[0];
    if(entry.callable) {
      kind='legacy_callable_declaration';
      if(entry.callable.asteriskToken) {
        kind='local_generator_declaration';error('entry_main_generator','生成器不是旧入口需要的JSON结果或其Promise。',entry.name);
      }
      const p=entry.callable.parameters;
      if(p.length!==1||!ts.isIdentifier(p[0]?.name)||p[0].initializer||p[0].dotDotDotToken)
        warn('legacy_entry_main_signature_unresolved','这里只确认一个简单上下文参数的 main；其它签名保留不完整标记。',entry.name);
    } else if(entry.value&&(ts.isClassDeclaration(entry.value)||ts.isClassExpression(entry.value)
      ||ts.isStringLiteralLike(entry.value)||ts.isNumericLiteral(entry.value)||ts.isObjectLiteralExpression(entry.value)
      ||ts.isArrayLiteralExpression(entry.value)||[ts.SyntaxKind.NullKeyword,ts.SyntaxKind.TrueKeyword,ts.SyntaxKind.FalseKeyword].includes(entry.value.kind))) {
      kind='local_noncallable_declaration';error('entry_main_noncallable','main 的声明不是可调用函数。',entry.name);
    } else warn('legacy_entry_main_unresolved','main 来自运行时表达式或别名，未确定实际入口值。',entry.name);
  }
  const mainSymbol=entry&&checker.getSymbolAtLocation(entry.name);
  if(mainSymbol?.declarations?.some(d=>d!==entry.name.parent))
    warn('legacy_entry_main_redeclared','main 还有同一作用域的声明，不能确认唯一入口绑定。',entry.name);
  const target=(node)=>{
    if(ts.isIdentifier(node))return checker.getSymbolAtLocation(node)===mainSymbol&&mainSymbol!==undefined;
    if(ts.isParenthesizedExpression(node)||ts.isSpreadElement(node)||ts.isSpreadAssignment(node))return target(node.expression);
    if(ts.isArrayLiteralExpression(node))return node.elements.some(target);
    if(ts.isObjectLiteralExpression(node))return node.properties.some(p=>ts.isShorthandPropertyAssignment(p)
      ?checker.getShorthandAssignmentValueSymbol(p)===mainSymbol&&mainSymbol!==undefined
      :ts.isPropertyAssignment(p)?target(p.initializer):ts.isSpreadAssignment(p)&&target(p.expression));
    if(ts.isBinaryExpression(node)&&node.operatorToken.kind===ts.SyntaxKind.EqualsToken)return target(node.left);
    return false;
  };
  function safeInitializer(node,depth=0) {
    if(!node)return true;if(depth>32)return false;
    if(fn(node)||ts.isStringLiteralLike(node)||ts.isNumericLiteral(node)||ts.isBigIntLiteral(node)
      ||ts.isRegularExpressionLiteral(node)||[ts.SyntaxKind.NullKeyword,ts.SyntaxKind.TrueKeyword,ts.SyntaxKind.FalseKeyword].includes(node.kind))return true;
    if(ts.isParenthesizedExpression(node))return safeInitializer(node.expression,depth+1);
    if(ts.isPrefixUnaryExpression(node)&&[ts.SyntaxKind.PlusToken,ts.SyntaxKind.MinusToken,ts.SyntaxKind.ExclamationToken,ts.SyntaxKind.TildeToken].includes(node.operator))return safeInitializer(node.operand,depth+1);
    if(ts.isArrayLiteralExpression(node))return node.elements.every(e=>!ts.isSpreadElement(e)&&safeInitializer(e,depth+1));
    if(ts.isObjectLiteralExpression(node))return node.properties.every(p=>!p.name||!ts.isComputedPropertyName(p.name))
      &&node.properties.every(p=>ts.isPropertyAssignment(p)?safeInitializer(p.initializer,depth+1):ts.isMethodDeclaration(p)||ts.isGetAccessor(p)||ts.isSetAccessor(p));
    return false;
  }
  for(const statement of source.statements) {
    const safe=ts.isFunctionDeclaration(statement)||ts.isEmptyStatement(statement)
      ||ts.isExpressionStatement(statement)&&ts.isStringLiteral(statement.expression)
      ||ts.isVariableStatement(statement)&&statement.declarationList.declarations.every(d=>safeInitializer(d.initializer));
    if(!safe)warn('legacy_entry_initialization_unresolved','顶层初始化或控制流可能改变入口或加载代码，静态检查不求值。',statement);
  }
  let possibleDynamicMain=false;
  const functions=[];
  const reference=node=>{
    const parent=node.parent;
    if(ts.isPropertyAccessExpression(parent)&&parent.name===node)return false;
    if((ts.isPropertyAssignment(parent)||ts.isMethodDeclaration(parent)||ts.isGetAccessor(parent)||ts.isSetAccessor(parent))&&parent.name===node)return false;
    if((ts.isVariableDeclaration(parent)||ts.isParameter(parent)||ts.isFunctionDeclaration(parent)||ts.isFunctionExpression(parent)||ts.isClassDeclaration(parent))&&parent.name===node)return false;
    return !(ts.isLabeledStatement(parent)||ts.isBreakStatement(parent)||ts.isContinueStatement(parent));
  };
  const dynamicCallee=(node,seen=new Set())=>{
    if(!node||seen.has(node))return true;seen.add(node);
    if(ts.isParenthesizedExpression(node))return dynamicCallee(node.expression,seen);
    if(ts.isCallExpression(node))return true;
    if(ts.isElementAccessExpression(node))return !ts.isStringLiteralLike(node.argumentExpression)&&!ts.isNumericLiteral(node.argumentExpression);
    if(ts.isIdentifier(node)) {
      const declaration=checker.getSymbolAtLocation(node)?.valueDeclaration;
      if(declaration&&ts.isVariableDeclaration(declaration)&&declaration.initializer&&!fn(declaration.initializer))
        return dynamicCallee(declaration.initializer,seen)||!ts.isIdentifier(declaration.initializer);
    }
    return false;
  };
  function visit(node) {
    const isFunction=ts.isFunctionLike(node);
    if(isFunction)functions.push(node);
    if(ts.isFunctionDeclaration(node)&&node.name?.text==='main'&&functions.length===1&&node.parent!==source)possibleDynamicMain=true;
    if(ts.isBinaryExpression(node)&&node.operatorToken.kind>=ts.SyntaxKind.FirstAssignment&&node.operatorToken.kind<=ts.SyntaxKind.LastAssignment) {
      if(ts.isIdentifier(node.left)&&node.left.text==='main'&&functions.length===0)possibleDynamicMain=true;
      if(target(node.left))warn('legacy_entry_main_reassigned','main 可能在运行时被重赋值，不能确认最终入口。',node.left);
    }
    if((ts.isPrefixUnaryExpression(node)||ts.isPostfixUnaryExpression(node))&&[ts.SyntaxKind.PlusPlusToken,ts.SyntaxKind.MinusMinusToken].includes(node.operator)&&target(node.operand))
      warn('legacy_entry_main_reassigned','main 存在运行时写入，不能确认最终入口。',node.operand);
    if((ts.isForInStatement(node)||ts.isForOfStatement(node))&&!ts.isVariableDeclarationList(node.initializer)&&target(node.initializer))
      warn('legacy_entry_main_reassigned','循环目标写入 main，不能确认最终入口。',node.initializer);
    if(ts.isReturnStatement(node)) {
      if(!functions.length)warn('legacy_entry_early_return','顶层 return 会绕过宿主追加的 main(ctx) 调用。',node);
    }
    if((ts.isAwaitExpression(node)||ts.isForOfStatement(node)&&node.awaitModifier)
      &&!has(functions.at(-1)??source,ts.SyntaxKind.AsyncKeyword))error('legacy_entry_await_unsupported','旧函数体和同步 main 不能使用 await。',node);
    if(ts.isYieldExpression(node)&&!functions.at(-1)?.asteriskToken)
      error('legacy_entry_yield_unsupported','旧函数体和普通 main 不能使用 yield。',node);
    if(ts.isMetaProperty(node)&&node.keywordToken===ts.SyntaxKind.ImportKeyword)
      error('legacy_entry_import_meta','旧函数体不是模块，不能使用 import.meta。',node);
    if(ts.isImportDeclaration(node)||ts.isExportDeclaration(node)||ts.isExportAssignment(node)||has(node,ts.SyntaxKind.ExportKeyword))
      error('legacy_entry_module_syntax','旧 .js 入口按函数体执行，不能使用静态 import/export；模块入口请用 .mjs。',node);
    if(ts.isWithStatement(node))warn('legacy_entry_dynamic_scope','with 改变名称查找，静态分析不能确定入口和依赖。',node);
    if(ts.isCallExpression(node)&&node.expression.kind===ts.SyntaxKind.ImportKeyword)
      warn('legacy_dynamic_import_unresolved','旧函数体协议不提供包内模块加载器，动态导入的目标及预算未完整分析。',node);
    if((ts.isCallExpression(node)||ts.isNewExpression(node))&&dynamicCallee(node.expression))
      warn('legacy_entry_dynamic_call_target','动态选择或生成的调用目标可能加载代码，静态依赖未完整覆盖。',node);
    if(ts.isIdentifier(node)&&reference(node)&&['eval','Function','require'].includes(node.text)) {
      const declaration=checker.getSymbolAtLocation(node)?.valueDeclaration;
      if(!fn(declaration)&&!(declaration&&ts.isVariableDeclaration(declaration)&&fn(declaration.initializer)))
        warn('legacy_entry_dynamic_code','发现求值、代码构造或 require 引用，静态依赖未完整覆盖。',node);
    }
    if(ts.isPropertyAccessExpression(node)&&['eval','Function','require','constructor'].includes(node.name.text)
      ||ts.isElementAccessExpression(node)&&ts.isStringLiteralLike(node.argumentExpression)&&['eval','Function','require','constructor'].includes(node.argumentExpression.text))
      warn('legacy_entry_dynamic_code','属性或别名可能获得动态代码/加载函数，静态依赖未完整覆盖。',node);
    if(ts.isIdentifier(node)&&node.text==='globalThis') {
      const parent=node.parent;
      if(!(ts.isPropertyAccessExpression(parent)&&parent.expression===node)
        &&!(ts.isElementAccessExpression(parent)&&parent.expression===node&&ts.isStringLiteralLike(parent.argumentExpression)))
        warn('legacy_entry_dynamic_global','动态访问全局对象不能确定代码依赖。',node);
    }
    ts.forEachChild(node,visit);
    if(isFunction)functions.pop();
  }
  try {visit(source);}
  catch {error('legacy_entry_analysis_limit','源码超过入口分析器可处理的复杂度。',source);}
  if(!candidates.length) {
    if(possibleDynamicMain)warn('legacy_entry_main_unresolved','main 仅出现在条件声明或运行时赋值中，不能确认入口。',source);
    else {kind='missing_legacy_main';error('legacy_entry_main_missing','旧 .js 入口须声明可调用的 main(ctx)，不要求 ESM 导出。',source);}
  }
  const passed=!diagnostics.some(d=>d.severity==='error');
  const dependencyDiagnostics=diagnostics.filter(d=>['legacy_entry_dynamic_code','legacy_entry_dynamic_scope','legacy_entry_dynamic_global','legacy_entry_dynamic_call_target','legacy_entry_initialization_unresolved','legacy_dynamic_import_unresolved'].includes(d.code));
  return {passed,entryProtocol:'legacy_function_body',mainExport:kind,diagnostics,dependencyDiagnostics,
    staticAnalysisComplete:passed&&kind==='legacy_callable_declaration'&&diagnostics.length===0,
    dependencyAnalysisComplete:dependencyDiagnostics.length===0,
    declarationAnalysisOnly:true,scope:'declaration_and_syntactic_dependencies_only',runtimeVerified:false,runtimeExecuted:false,filesRead:false,filesModified:false};
}
