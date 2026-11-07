# RaiShot 截图与托盘示例

动作包 `examples.raishot` 提供 Node 和同步 Script 两个入口，复用 OmniTrigger 已有的 RaiShot 常驻协议和有界进程采集 API。它是开发集成示例，尚未接入默认扩展安装流程。

| `launch_mode` | 当前行为 |
| --- | --- |
| `region`（默认） | 已有常驻时请求区域截图；否则准备依赖并启动 `raishot.exe --region`。 |
| `tray` | 已有常驻时直接返回；否则准备依赖并启动 `raishot.exe --tray`。 |
| `verify` | 跳过常驻查询，完整核对依赖清单、文件大小、SHA-256 和运行库声明，不启动 RaiShot。缺少依赖时先准备固定便携包。 |

常驻分支不访问依赖目录、不下载文件、不启动另一个程序。身份变化或投递结果不明时直接报错，不退回启动或自动重试。`queued` 只表示消息排队，`launched` 只表示取得进程 ID；二者均不证明截图完成。

## 开发输入

| 输入 | 要求 |
| --- | --- |
| `launch_mode` | `region`、`tray`、`verify`；缺省为 region。忽略首尾空格和大小写，空白或未知模式报错。 |
| `quicker_in_param` | 可选兼容参数；非空字符串优先于 launch_mode。 |
| `powerShellProgram` | verify 和无常驻冷启动必填：已定位的本机 x64 PowerShell `.exe` 绝对路径。常驻 region/tray 无需此输入。 |
| `packageDirectory` | 可选：本机已有 RaiShot 包目录的绝对路径；不传时查找共享安装位置，需要时准备固定包。 |

动作不搜索 PATH，也不提供系统盘或解释器目录的隐含默认值。默认安装组合根尚未自动注入解释器路径，不能将本示例称为默认安装或全部冷启动已经完成。

以下是显式校验已有包的开发调用；路径应替换为本机已确认的位置。Script 入口将 action_id 改为 `script`：

```json
{
  "package_id": "examples.raishot",
  "action_id": "node",
  "action_version": 1,
  "input": {
    "launch_mode": "verify",
    "powerShellProgram": "D:\\Tools\\PowerShell\\pwsh.exe",
    "packageDirectory": "D:\\Tools\\RaiShot\\0.4.18"
  }
}
```

## 依赖校验

最低包版本为 0.4.18，支持 `rustshot` 和 `cea.rustshot` 清单标识，不改写包名。新下载使用 [官方 v0.4.18 便携包](https://github.com/ceastld/raishot/releases/tag/v0.4.18)：`RaiShot.0.4.18.0.zip`，29,657,178 字节，SHA-256 `a56bc118e55ea53c865987a7bd28db80395ce4aa61653f3e70ca412f07238490`。

默认只查 `%LOCALAPPDATA%/Programs/RaiShot` 及最多 64 个版本子目录，复用最高可用版本。新下载安装到独立的 0.4.18 子目录；已有同版本目录不覆盖，损坏时报错。程序、运行库和许可文件完整保留；不操作既有托盘或 RaiShot 用户数据，不注册任务和自动启动项。本示例不改变应用原有的内置 RaiShot 安装器。

普通启动只做清单结构与文件大小检查。verify 核对全部声明文件的 SHA-256、必要程序和运行库引用；拒绝不安全文件名、重复项、链接、云盘目录及云端占位文件。新安装保留 `.raishot-source.zip`，完整校验会重新核验该 ZIP，再比对包内清单，返回 `archive_authenticated:true`。外部安装没有固定归档时只证明清单与文件一致性，返回 `archive_authenticated:false`。

helper 通过 `process.capture` 执行，调用上限 60 秒，下载上限 40 秒；超时或失败不返回部分成功结果，也不自动重试。强制终止 helper 可能留下独立暂存目录，但不会覆盖正式安装。Node 启动显式使用 `windowsHide:false` 和包目录 cwd；Script 使用两参数启动并继承宿主 cwd。关键 EXE/DLL/图标从 EXE 同级目录、System32 或内嵌资源定位，但相对的 RUSTSHOT_DATA_DIR 和部分保存回退路径仍可能随 cwd 变化。

## 验证边界

Windows x64 上已用真实官方 0.4.18 包通过两个运行时的普通 CLI verify：完整校验、归档认证、正常退出且无残留采集作业。该过程没有启动 RaiShot 或查询常驻。真实区域截图、托盘冷启动及完整界面流程尚未验收。已有常驻可能属于旧版本；包校验通过不证明现有实例的版本。

纯消费者测试与类型检查命令如下，不启动 RaiShot、解释器或 GUI：

```text
node --test tests/raishot_consumer.test.mjs
node tools/script-sdk-check/node_modules/typescript/lib/tsc.js -p examples/actions/raishot/jsconfig.json
```

类型声明快照来自本仓库公开提交 `56e35e2bf6a9a54210dde1ae038da907bd3cd031`。TypeScript 使用现有 `tools/script-sdk-check` 开发依赖，不新增运行时依赖。
