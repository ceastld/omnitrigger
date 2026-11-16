// 仅检查已提供的 JS AST；不解析依赖、不读取标准库、不生成或转译源码。
import ts from 'typescript';

export function javascriptAnalysisProgram(source) {
  const host={
    getSourceFile:file=>file===source.fileName?source:undefined,
    getDefaultLibFileName:()=>'',
    getCurrentDirectory:()=>'',
    getCanonicalFileName:file=>file,
    useCaseSensitiveFileNames:()=>true,
    getNewLine:()=> '\n',
    fileExists:()=>false,
    readFile:()=>undefined,
    writeFile:()=>{},
  };
  // createSourceFile 的 parseDiagnostics 不包含 JS 文件中的 TS 专有语法诊断。
  return ts.createProgram([source.fileName],{allowJs:true,noLib:true,noResolve:true,noEmit:true,types:[]},host);
}

export function javascriptSyntaxDiagnostics(source) {
  return javascriptAnalysisProgram(source).getSyntacticDiagnostics(source);
}
