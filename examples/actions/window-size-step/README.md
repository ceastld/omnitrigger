# 窗口大小增减

普通 Node 和同步 Script 入口使用相同的业务叶。`increase`/`decrease` 分别将指向窗口的可见宽高同时加减一步，左上角保持不变。默认步长50，接受10–200整数；`trigger` 须明确为 `AdvancedMouseAction` 或 `PowerKeys`。

调用示例：

```json
{"package_id":"examples.window-size-step","action_id":"increase","action_version":1,"input":{"trigger":"AdvancedMouseAction","step":50,"targetPolicy":"original-only"}}
```

同步 Script 使用 `increase-script`/`decrease-script`。默认 `original-only`：鼠标指向根窗口与原捕获目标不同则停止。显式 `handoff-pointing-root` 还需给出 `handoff:{validForMs:5000,confirmTimeoutMs:1000}`；确认后只携带同一租约调用 For，不重捕获、不延长期限、不退回原窗口。

每次先取得稳定的只读几何及原始 `className`，按来源固定模式在业务层判断。普通 Unicode 类名可以继续调整；本机 .NET Regex 差分显示 long-s（`ſ`）不参与固定模式折叠，Kelvin（`K`）则存在文化差异。只有完整黑名单匹配确实依赖 Kelvin 折叠时返回 `source_regex_culture_unavailable`，不整体拒绝非ASCII类名，也不宣称完全复刻所有 .NET 文化。

平台保持原有三个完整类名守卫，专用 `window_class_blocked` 仅表示明确命中该守卫。初始 geometry 读取中的这一结构化拒绝映射为正常无操作，`noOpReason=exact_platform_class`；快照业务模式命中则为 `source_pattern`。不匹配错误正文，不将 `unsupported_target`、身份、期限或其他 Native 错误当作无操作，也不吞掉还原后或写入阶段的错误。

最大化窗口显式请求还原一次；仍最大化则返回 `restore_pending`。增减使用还原前已经冻结的尺寸和位置，不随还原后新尺寸重算。成功仅返回 `size_submitted`，表示系统接纳一次写入请求，`windowChangeConfirmed` 始终 false。

非正尺寸、超出现有 SDK 的1–65535尺寸限制或右下边界溢出均明确停止；不钳制、不声称完成原动作的负尺寸特殊分支。取消、超时、交接未确认、坏回执和 unknown 均不自动重试或回滚。

大小步进在来源双击/时间戳 State 分支之前就退出，因此本包不读写该时间戳、不 stopOther、不拖动或注入按下释放，也未迁移原设置表单及保存配置。包的 `single_instance:false` 保留来源声明，不扩展应用已有的再入准入政策。

当原目标不是指向根窗，现有 For 协议要求先明确交接前台，再读取 B 的几何和类名。这比来源早期直接观察 B 多一次显式副作用；后续停止仍可能已经改变前台，回执保留 unknown，不能声称该时序完全等价。

## 独立作者检查与 SDK 配对

本包需要带有必需 WindowGeometry.className 字段和精确 window_class_blocked 拒绝语义的配套 SDK/宿主。SDK 与宿主应共同更新；仅替换声明不能为旧宿主增加元数据，严格 DTO 的新旧版本也不能混用。静态检查通过不表示宿主能力或真实窗口效果已验证。

使用 Node.js 22 或更高版本，在本包目录运行 `npm ci --ignore-scripts` 和 `npm run check`。有完整 npm 缓存时可加 --offline。检查固定使用本包 TypeScript 5.9.3 与静态入口检查器，Node API 返回 Promise，Script API 同步返回；Script 不加载 DOM 或 Node 环境声明。许可见 THIRD_PARTY_NOTICES.md。
