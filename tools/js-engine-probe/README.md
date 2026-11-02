# Rust 脚本宿主体积实验

状态：独立可运行的实验，尚未接入 OmniTrigger 动作执行或插件安装。目标是测量 Rust 实现的最小 JS 宿主在 Windows 上的实际体积，给可选脚本插件的选型提供依据。

## 两条路线

| 引擎 | 实现 | 版本与选择 |
| --- | --- | --- |
| [Boa](https://github.com/boa-dev/boa) | 纯 Rust JS 引擎 | 固定 0.21.1，兼容项目 Rust 1.90；0.22.0 要求 Rust 1.91。关闭默认可选功能，不启用 Intl / Temporal。 |
| [rquickjs](https://github.com/DelSkayn/rquickjs) / QuickJS-NG | Rust 宿主，静态编译 C 引擎 | 固定 rquickjs 0.14.0，只启用 `std`，不启用模块加载器、宏或异步 Rust 适配。 |

两者使用同一份宿主代码和 JSON 接口，不需要安装 Node 或 Electron。引擎依赖通过独立 `Cargo.lock` 固定，不进入主程序 workspace。许可和未来分发要求见[第三方通知](../../THIRD_PARTY_NOTICES.md)。

## 2026-11-02 Windows x64 实测

| 引擎 | 普通 Release EXE | 优化并静态链接 CRT 的 EXE | 优化版仅 EXE 的 ZIP |
| --- | --- | --- | --- |
| Boa（纯 Rust） | 11,809,280 字节 / 11.26 MiB | 3,456,512 字节 / 3.30 MiB | 1,480,233 字节 / 1.41 MiB |
| Rust + QuickJS-NG | 1,389,056 字节 / 1.32 MiB | 1,206,272 字节 / 1.15 MiB | 662,527 字节 / 0.63 MiB |

纯 Rust 宿主可以控制在约 3.30 MiB，轻量插件路线可行；体积优先时，QuickJS-NG 方案约为 1.15 MiB。两份优化版都已运行并检查 PE 导入，只依赖 Windows 自带 DLL。这里只包含下文的同步 JS 和 JSON 协议，加入 TS、模块系统或桌面 SDK 后应重新测量，尚不确定最终插件体积。

完整数据见[本机测量快照](measurements.windows-x64.json)，包含每个 EXE 的哈希与 DLL 导入。耗时包含进程创建与退出，受本机负载影响，不用于比较引擎执行性能；本表为此次本机数据，CI 会生成另一份实测报告，不要求字节相同。

## 运行与复测

需要 Windows x64、Rust 1.90 MSVC、Visual Studio C++ 构建工具和 PowerShell 7。从本目录执行：

```powershell
./measure.ps1
```

脚本分别构建 `release` 与 `small` 两个配置，并从 Cargo 回执取得 EXE 路径。`release` 保留默认动态 CRT；`small` 使用 `opt-level = "z"`、LTO、一个代码生成单元、移除符号和 `panic = "abort"`，另加 `+crt-static`，以测量无需另配 VC 运行库的宿主。未使用 UPX。两组同时改变优化和 CRT 链接方式，不能把差异全部归因于优化。结果保存于被忽略的 `results/`：

- `boa-release.exe` / `boa-small.exe`。
- `quickjs-release.exe` / `quickjs-small.exe`。
- 只含相应 EXE 的 ZIP，用于比较压缩体积，不是完整发行包。
- `measurements.json`：精确字节、SHA-256、工具链、锁文件哈希、PE 导入 DLL 及进程往返耗时。`small` 额外检查没有 VC 运行库 DLL 导入，仍依赖 Windows 自带组件。

每次采样都创建一个新进程，写入输入、执行 JS、读取回执并等待退出。首次往返单列，后续 11 次取中位数；这不是引擎内部执行时间，也不是清除操作系统文件缓存后的冷启动。构建配置不同、杀毒软件和机器负载都会影响耗时。两种引擎的限制策略不同，本实验不用于比较算法性能。

示例（PowerShell 7）：

```powershell
'{"script":"return { total: ctx.input.items.reduce((a,b) => a+b, 0) };","input":{"items":[1,2,3]}}' |
    ./results/boa-small.exe
```

输出：

```json
{"ok":true,"result":{"total":6}}
```

入口为同步函数体，通过 `ctx.input` 读入 JSON 对象，`return` 返回 JSON 值。失败返回 `{"ok":false,"error":"原因"}` 并以退出码 1 结束。支持的语言语法以实际引擎为准；回归覆盖箭头函数、可选链、空值合并、BigInt 运算后转字符串、中文及特殊字符。拒绝 Promise、无返回值、语法错误和不能 JSON 序列化的返回值。

## 边界与后续

- 当前只有 JS，没有 TS 转译、ESM / npm 加载、Node API、文件 / 网络 / 桌面 SDK、编辑器或插件安装器。体积结论只覆盖这里实现的同步宿主，不能当作完整脚本平台体积。
- 请求最多 256 KiB，最终结果最多 64 KiB。结果限制在引擎序列化之后检查，不是统一的峰值内存限制。
- Boa 配置循环迭代和递归限制，没有宿主墙钟超时和堆内存上限；QuickJS 配置 32 MiB 引擎内存、512 KiB 栈和约 2 秒中断期限。内存限制不包含 Rust 宿主，均不宣称安全沙盒。
- 正式插件应通过外层进程监测实现统一超时和取消，并沿用 `ActionCall` / `ActionRuntime`，再按业务需要增加能力。无需先替换现有 Node 动作。
- 如需 TS，后续单独测量 Rust 转译器带来的增量；也可在编辑或保存时转译，让执行插件只保留 JS 引擎。本次未测，不预估其体积。

真实进程回归测试：

```powershell
cargo test --release --locked --no-default-features --features boa
cargo test --release --locked --no-default-features --features quickjs --target-dir target/quickjs
```

CI 对两个引擎分别执行测试与 Clippy，再构建四份 EXE，仅保存测量 JSON。主应用继续执行原有完整检查及 Linux 核心测试。
