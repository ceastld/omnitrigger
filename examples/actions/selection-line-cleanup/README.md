# 清理选区行（实验）

读取动作捕获的原窗口选区，提供两种操作：

- `remove-empty-lines`：删除长度为零的行，保留只含空格或制表符的行。
- `remove-white-space-lines`：删除整行均为空白字符的行，使用 .NET 字符分类集合；U+0085 属于空白，U+FEFF 保留。保留行的首尾空白不变。

两种操作均识别 CRLF、CR 和 LF，结果用 CRLF 连接，不保留终止换行。输入只接受可选的 `operation`，默认删除空行。Node 异步入口为 `node`，轻量同步 ESM 入口为 `script`。

源选区显式读取最多 8192 个 UTF-16 单元，拒绝截断。首版待发送结果最多 128 个单元；超过后返回 `replacement_too_long / keyboard.text / not_started`，不复读或发送，不截断结果。源上限没有改变默认选区 API 的 128 单元上限，也不表示能在发送期限内输入 8192 单元。

无选区、结果不变或结果为空时只读一次，不发送。其他情况再次读取并逐字比较原文，相同后发送一次。两次读取和发送之间没有选区锁；相同文本不能证明选区范围没有移动。失焦、超时或取消后不重试、不补抓前台，不使用剪贴板。

`replacement_submitted` 只表示输入请求被系统接纳。已输入的文字无法回滚，`unknown` 错误可能已产生部分输入，不能据此重试。

## 验证范围与已知限制

Node 和 QuickJS 已在自建、完成输入预处理初始化的 WinForms 文本框各验收 18 项，包括实际编辑、129 单元源文本转换、8192 单元空白压缩为短结果、128/129 单元 emoji 输出边界、源超限拒绝及切到第三窗口后拒绝发送。验收使用合成文本。

新建且尚未完成输入预处理初始化的 WinForms 控件已复现首字符异常，仍未修复；在这类目标中暂缓使用。本示例保持实验状态，不自动安装或启用个人绑定。其他应用的选区提供器和输入处理仍需单独验收。

四个 `call-*.json` 文件分别选择操作和运行时，实际运行会修改选区。纯契约检查使用合成数据和真实 SDK 工厂，不访问窗口：

```powershell
node --test examples/actions/selection-line-cleanup/recipes.test.mjs
node tools/script-sdk-check/node_modules/typescript/lib/tsc.js -p examples/actions/selection-line-cleanup/jsconfig.json
node tools/script-sdk-check/check-project-types.mjs examples/actions/selection-line-cleanup
```

## 独立作者检查

在本工程目录中使用 Node.js 22 或以上版本执行：

```powershell
npm ci --ignore-scripts
npm run check
```

TypeScript 5.9.3 仅为开发依赖，版本与完整性由本包锁文件固定。检查使用本包声明与静态入口检查器，不需要仓库的 crates、tools 或 node_modules，不导入或执行作者动作；保持 strict、skipLibCheck:false、NodeNext 与 types:[]，只加载 ES2022，不向 Script 加入 DOM 或 Node 全局。

命名为 sdk/api.d.ts 的文件仍是同步 Script 声明；Node 入口使用 types/api.d.ts 的异步声明。开发清单不设置 package.type，保留既有 .mjs 模块与 NodeNext 声明的解释。类型通过不代表选区已经复制或替换，也不能替代原目标、取消与宿主能力验证。
