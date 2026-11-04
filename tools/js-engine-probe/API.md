# 首批脚本 API

状态：可运行的同步原型，不是正式插件。两种引擎加载同一份 `src/sdk.js`，通过 JSON 桥接到同一份 `src/api.rs`。剪贴板和打开目标复用 `omnitrigger-windows`，不安装 Hook，不访问 GPUI。`ctx.apiVersion` 为 `1`。[类型声明](api.d.ts)用于编辑器补全，不提供 TS 转译。

通过 OmniTrigger 的 `script` 动作运行时接入时，文件声明 `function main(ctx)`；适配层将它转为本工具的函数体请求，文件本身保持标准 JavaScript。`ctx.packageRoot` 为动作包目录，`ctx.invocation` 包含十进制字符串调用身份。独立 CLI 可以省略这些元数据，此时两项为 `null`。成功与失败响应均带有 `schema_version: 1`。

| API | 行为 |
| --- | --- |
| `ctx.fs.readText(path)` | 读取绝对路径的 UTF-8 文本，最多 64 KiB。 |
| `ctx.fs.writeText(path, text, {overwrite})` | 默认创建新文件，目标存在则报错；只有显式 `overwrite:true` 才覆盖，不自动创建目录。 |
| `ctx.process.run(program, args, {timeoutMs})` | 程序和参数数组分别传递，不隐式经过 Shell。返回 `{pid, exitCode, success}`；默认等待 1000 毫秒，允许 1–1000 毫秒。非零退出由脚本判断，stdout / stderr 不采集。 |
| `ctx.process.start(program, args)` | 创建独立程序后返回 `{pid}`，程序不随脚本结束关闭。 |
| `ctx.clipboard.readText()` / `writeText(text)` | Windows Unicode 文本剪贴板；写入会替换内容。 |
| `ctx.open(target)` | 系统默认程序打开 http、https、mailto、about 或本地绝对路径；成功只表示系统接纳请求。 |

原生错误转换为 JS 异常，可在 `catch (error)` 中读取 `error.code`，例如 `invalid_arguments`、`file_read_failed`、`file_write_failed`、`process_start_failed` 和 `process_timeout`。通过 SDK 每个脚本最多调用 32 次 API，单个桥接请求最多 256 KiB，文本 API 最多 64 KiB。SDK 计数是防误用措施，不是安全隔离。

## 文件清理动作

[完整入口](examples/clean-file.js)读取文本、清除行尾空白、生成新文件，使用普通 JS 完成处理：

```js
const text = ctx.fs.readText(ctx.input.source);
const cleaned = text.replace(/\r\n?/g, '\n').replace(/[\t ]+$/gm, '');
ctx.fs.writeText(ctx.input.destination, cleaned);
return { destination: ctx.input.destination, characters: cleaned.length };
```

从本目录执行（需提供真实输入文件和不存在的输出路径）：

```powershell
@{
    script = Get-Content ./examples/clean-file.js -Raw
    input = @{ source = 'D:/Temp/input.txt'; destination = 'D:/Temp/cleaned.txt' }
} | ConvertTo-Json -Depth 8 -Compress | ./results/boa-small.exe
```

## 剪贴板清理动作

[完整入口](examples/clean-clipboard.js)读取文本，清理行尾空白，写回剪贴板：

```js
const text = ctx.clipboard.readText();
const cleaned = text.replace(/\r\n?/g, '\n').replace(/[\t ]+$/gm, '');
ctx.clipboard.writeText(cleaned);
return { before: text.length, after: cleaned.length };
```

将上述入口作为请求的 `script`、`input` 设置为 `{}` 后执行，会真实修改剪贴板。

## 当前执行边界

- API 同步运行在独立宿主进程中，不进入输入 Hook。原生能力由 Rust 提供，JS SDK 和引擎适配分别演进。
- API 使用当前用户权限，文件路径与 JSON 桥接不是沙盒。文件覆盖、剪贴板修改和外部程序副作用不会回滚，写入失败可能留下部分内容。
- `run` 的等待期限从进程创建返回后计算。超时会请求终止并等待直接子进程，不保证它自行创建的后代全部结束。`start` 的独立程序始终保留。
- JS 执行限制不能中断正在阻塞的原生文件或外壳调用，目前没有覆盖所有原生调用的统一墙钟取消。正式接入需要外层执行器的超时与取消管理。
- 键盘、鼠标和窗口 API 尚未开放。后续应携带原目标身份，复用已有注入与释放收尾协议，不允许任意裸输入绕过这些边界。

自动测试对临时文件执行真实 JS → Rust → 文件读写，验证默认禁止覆盖、显式覆盖、结构化错误、真实程序退出码、独立启动和超时。剪贴板读取另在本机只读验证；测试不写用户剪贴板，也不打开外部窗口。底层剪贴板和打开目标的替身测试沿用主 workspace 的原生适配测试。
