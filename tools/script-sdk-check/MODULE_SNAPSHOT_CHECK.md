# 检查轻量模块导入快照

`analyze-script-modules.mjs` 补充类型检查之外的轻量宿主加载限制。它只接收调用方明确提供的源码文本，不枚举目录、不读取作者文件、不导入作者模块，也不启动宿主。

```javascript
import {analyzeScriptModules} from './analyze-script-modules.mjs';
const result = analyzeScriptModules({
  entry: 'main.mjs',
  sources: [
    {path: 'main.mjs', text: 'import {value} from "./value.js"; export function main(){return value;}'},
    {path: 'value.js', text: 'export const value = 1;'},
  ],
});
console.log(JSON.stringify(result));
```

使用仓库已有 TypeScript 开发依赖，没有增加依赖。运行纯文本测试：

```powershell
node --test tools/script-sdk-check/analyze-script-modules.test.mjs
```

检查项以轻量宿主现有 `modules.rs` 为依据：

- 模块入口是包内 `.mjs` 路径，静态导入及再导出需要显式 `.js`／`.mjs`。
- 相对路径不能越过包根，不补全目录或扩展名；拒绝包名、Node 内置模块、URL、查询片段、反斜杠及 Windows 设备名。
- 单模块最多 128 KiB UTF-8，静态加载图最多 64 个模块、合计 1 MiB。循环及重复引用只计算一次。
- 调用方快照最多 256 个文件、合计 8 MiB；未被静态引用的源码不计入加载图。路径必须是规范相对路径，且与快照键精确对应。

结果包含 `modules`、`edges`、`totalBytes` 和诊断位置。`passed` 只表示没有这些静态检查错误。源文件语法错误或分析复杂度失败会明确拒绝。

非字面动态导入产生 `dynamic_import_unresolved` 警告；字面动态导入只检查路径，并产生 `dynamic_import_deferred` 警告。其目标子图和字节预算未计入静态图，所以 `staticGraphComplete` 为 `false`。存在静态错误时，该字段同样为 `false`。

缺少快照文件表示调用方未提供该文件，不证明磁盘文件缺失。该模块不校验磁盘链接、云端占位属性、文件权限或真实路径大小写；读取实际工程的调用方仍必须先执行元数据检查。它也不验证运行时 API 能力、原生桥接、目标软件效果，不能替代现有类型检查、静态动作引用检查或显式运行验收。`runtimeVerified` 始终为 `false`。


## 具名入口声明

`analyze-script-entry.mjs` 接收 `{file:'main.mjs', text:'…'}`，只检查 `main` 具名导出及其声明形式。普通或 async 函数、箭头函数和可静态解析的局部别名可被找到；仅默认导出、未导出的局部 main、明确数字/对象/class 的入口及生成器会产生诊断。import 别名、再导出、解构和运行时表达式保留为未确定警告，不读取依赖或求值。

`mainExport` 只描述静态声明。即使找到函数，也不证明初始化副作用或重赋值之后仍为可调用值，不验证同步返回、JSON 结果、SDK 参数或目标业务。`declarationAnalysisOnly` 为 true，`runtimeVerified` 始终为 false。

运行其纯文本测试：

```powershell
node --test tools/script-sdk-check/analyze-script-entry.test.mjs
```

## 旧函数体入口

`analyze-script-entry.mjs` 同样接收 `.js` 文本，按宿主旧函数体协议识别唯一普通或 async `main`，不要求 ESM 导出。函数声明、函数表达式或箭头函数变量绑定可得到 `mainExport:"legacy_callable_declaration"`；结果另带 `entryProtocol:"legacy_function_body"`、`staticAnalysisComplete` 和 `dependencyAnalysisComplete`。

仅一个简单标识符上下文形参可确认完整签名。缺失、重复、明确非函数或生成器入口会被拒绝；别名值、条件声明、其它签名、重绑定或不确定顶层初始化保留不完整警告。静态 import/export、import.meta 和同步作用域中的 await 等不符合该协议，返回定位诊断。动态 import、eval、Function、require、动态调用或全局查找使依赖覆盖不完整。分析不求值表达式，不证明任意值流、JSON 结果或实际宿主能力。

旧入口的图分析须显式传 `protocol:'legacy_function_body'`，入口路径为 `.js`。图只覆盖所给入口；静态模块语法会拒绝，动态导入只标记不完整及可确认的路径，不补读目标文件或计入静态加载预算。JSDoc 类型导入不生成运行时依赖边。默认协议仍是 `.mjs` 模块图，既有路径与容量约束保持有效。

```powershell
node --test tools/script-sdk-check/analyze-legacy-entry.test.mjs
```
