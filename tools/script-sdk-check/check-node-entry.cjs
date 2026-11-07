// @ts-nocheck -- 生成的开发检查器不参与作者工程的 checkJs。
// Node 工程的静态入口检查；TypeScript 读取声明，不导入或执行作者模块。
const {readFileSync} = require('node:fs');
const {isAbsolute, relative, resolve} = require('node:path');

function checkNodeEntry({projectDirectory, typescript:ts}) {
  const root = resolve(projectDirectory);
  const manifest = JSON.parse(readFileSync(resolve(root, 'action.json'), 'utf8'));
  if (manifest.schema_version !== 1 || !Array.isArray(manifest.actions) || manifest.actions.length > 64) {
    throw Object.assign(new Error('动作清单无效。'), {code:'node_entry_manifest_invalid'});
  }
  const actions = manifest.actions.filter(action => action?.runtime === 'node');
  if (!actions.length) throw Object.assign(new Error('清单没有 Node 动作。'), {code:'node_entry_missing'});
  const entries = actions.map(action => {
    if (typeof action.entry !== 'string' || !action.entry || action.entry.includes('\0') || isAbsolute(action.entry)) {
      throw Object.assign(new Error('Node 入口必须位于动作包内。'), {code:'node_entry_path_invalid'});
    }
    const file = resolve(root, action.entry), name = relative(root, file);
    if (name === '..' || name.startsWith('../') || name.startsWith('..\\') || isAbsolute(name)) {
      throw Object.assign(new Error('Node 入口必须位于动作包内。'), {code:'node_entry_path_invalid'});
    }
    return file;
  });
  const configFile = resolve(root, 'jsconfig.json');
  const config = ts.readConfigFile(configFile, ts.sys.readFile);
  if (config.error) throw Object.assign(new Error('无法读取 jsconfig.json。'), {code:'node_entry_config_invalid'});
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, root);
  if (parsed.errors.length) throw Object.assign(new Error('jsconfig.json 配置无效。'), {code:'node_entry_config_invalid'});
  const program = ts.createProgram([...new Set([...parsed.fileNames, ...entries])], {...parsed.options, noEmit:true});
  const checker = program.getTypeChecker(), diagnostics = [];
  // host 等待顶层 Promise，再以 output ?? null 写入 JSON 响应；不要求原值逐字段都是 JSON。
  // 这里只拒绝明确的顶层不支持类型，不把未知类型或对象序列化视为运行时验证。
  const resultState = type => {
    if (!type || type.flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown | ts.TypeFlags.TypeParameter
        | ts.TypeFlags.IndexedAccess | ts.TypeFlags.Conditional)) return 'unresolved';
    if (type.isUnion()) {
      const states = type.types.map(resultState);
      return states.every(state => state === states[0]) ? states[0] : 'unresolved';
    }
    // 自定义序列化可改变函数等对象的输出；静态检查不执行 toJSON。
    if (type.getProperty('toJSON')) return 'unresolved';
    if (type.flags & (ts.TypeFlags.BigIntLike | ts.TypeFlags.ESSymbolLike)
        || checker.getSignaturesOfType(type, ts.SignatureKind.Call).length
        || checker.getSignaturesOfType(type, ts.SignatureKind.Construct).length) return 'nonjson';
    return 'unchecked';
  };
  for (const file of new Set(entries)) {
    const source = program.getSourceFile(file), name = relative(root, file).replaceAll('\\', '/');
    const add = (code, severity, message, node) => {
      const point = source && node ? source.getLineAndCharacterOfPosition(node.getStart(source)) : undefined;
      diagnostics.push({file:name, code, severity, message, ...(point ? {line:point.line+1, column:point.character+1} : {})});
    };
    if (!source) { add('node_entry_unreadable', 'error', '无法读取清单中的 Node 入口。'); continue; }
    // CommonJS 的合成 default 由 Node 提供；不把 TS 的导出列表当成其运行时命名空间。
    if (file.endsWith('.cjs') || source.commonJsModuleIndicator) {
      add('node_entry_unresolved', 'warning', 'CommonJS 入口的运行时 default 未由此 ESM 检查确认。'); continue;
    }
    const moduleSymbol = checker.getSymbolAtLocation(source);
    const entry = moduleSymbol && checker.getExportsOfModule(moduleSymbol).find(symbol => symbol.name === 'default');
    if (!entry) { add('node_entry_default_missing', 'error', 'Node 动作入口必须 default 导出函数。'); continue; }
    const location = entry.valueDeclaration ?? entry.declarations?.[0] ?? source;
    const type = checker.getTypeOfSymbolAtLocation(entry, location);
    const signatures = checker.getSignaturesOfType(type, ts.SignatureKind.Call);
    if (signatures.length) {
      const states = signatures.map(signature => resultState(checker.getAwaitedType(checker.getReturnTypeOfSignature(signature))));
      if (states.every(state => state === 'nonjson')) {
        add('node_entry_result_nonjson', 'error', 'Node 入口等待后的返回类型明确不能生成所需 JSON output（函数、symbol 或 bigint）。', location);
      } else if (states.some(state => state === 'nonjson' || state === 'unresolved')) {
        add('node_entry_result_unresolved', 'warning', 'Node 入口结果的静态类型或自定义序列化未确定，JSON 响应仍需运行时确认。', location);
      }
      continue;
    }
    if (type.flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown) || type.isUnionOrIntersection()) {
      add('node_entry_unresolved', 'warning', 'default 的静态类型未确定，仍需运行时确认它可以调用。', location);
    } else add('node_entry_default_noncallable', 'error', 'Node 动作的 default 导出不是可调用函数。', location);
  }
  return {passed:!diagnostics.some(item => item.severity === 'error'), diagnostics,
    entries:entries.length, noEmit:true, authorCodeExecuted:false, runtimeVerified:false};
}

module.exports = {checkNodeEntry};
if (require.main === module) {
  try {
    const result = checkNodeEntry({projectDirectory:process.argv[2] ?? '.', typescript:require('typescript')});
    console.log(JSON.stringify(result));
    if (!result.passed) process.exitCode = 1;
  } catch (error) {
    console.error(JSON.stringify({passed:false, error:{code:error.code ?? 'node_entry_check_failed', message:error.message}, authorCodeExecuted:false}));
    process.exitCode = 1;
  }
}
