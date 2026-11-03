# 离线动作迁移评估

`scripts/plan_action_migration.py` 将已有的 Quicker V1 配置、完整动作目录、使用统计和当前 OmniTrigger 绑定合并成一份本机评估清单。它只读 JSON，不连接源程序、不修改配置、不启用绑定，也不执行动作。

需要 Python 3。输入文件应放在本机忽略目录，例如 `.tmp/migration/`，不要将个人动作正文、使用记录或评估清单加入版本控制。

```powershell
python scripts/plan_action_migration.py `
  --source .tmp/migration/source.json `
  --catalog .tmp/migration/catalog.json `
  --usage .tmp/migration/usage.json `
  --counters .tmp/migration/counters.json `
  --current .tmp/migration/bindings.json `
  --out .tmp/migration/plan.json
```

输出文件必须尚不存在。控制台仅输出条目数量；详细规则和文件路径保留在本机输出中。每份输入的 SHA-256 记录在 `evidence`，便于后续核对快照是否改变。

## 输入格式

- `source`：含 `PowerKeys` 字典、`MouseActions` 数组及可选 `ExeSettings` 字典的 V1 配置快照。每个前缀包含 `Key`、`IsEnabled`、`KeepOriginKeyFunc`、`BlackList`、`KeyActions`；规则保留原字段。应用配置中的 `MouseActions` 同样纳入。只读导出的 `InputSettings` 用于确认单按恢复超时及 `DefaultModifiedKeyDownDelay`；缺少后者时不能确认快捷键的输入时序。
- `catalog`：完整的 `payload.items`，或直接包含 `items` 的对象。每项含 `id`、`title`、`actionType`。目录的 `hasMore` 必须为假，`totalCount` 必须与条目数一致；分页结果应来自同一个 `snapshotId`。
- `usage`：`Days30` 和 `Days0` 各含 `IsSuccess: true` 与 `Data: { "动作ID": 次数 }`。`Days0` 保留为历史接口原值；工具不推断具体统计起始日期。
- `counters`：已确认的功能计数标识数组，形如 `[{ "Method": "功能名", "Ids": ["计数ID"] }]`。这些 ID 不作为动作排名。工具不猜测未知 ID 的用途。
- `current`：当前 OmniTrigger 的绑定配置，含 `bindings` 数组。

## 筛选及结果含义

筛选所有启用的扩展热键和鼠标规则，再加入近 30 天与历史统计各前 20 项；动作引用按 ID 去重。排名先排除已确认的功能计数，然后取前 20 项。未在本机目录中找到的 ID 仍占据排名，并标记为待解析，不自动替换成下一项。

只有历史计数、没有近期调用且没有启用绑定引用的动作标记为暂缓。直接发送快捷键的规则不依赖动作 ID 的使用计数，不能因缺少计数被判为闲置。所有暂缓项保留原数据，不自动删除。

`operation.native_action` 目前仅转换 V1 `Keystroke` 的 `modifier|VK` 数据，产生独立的 OmniTrigger `send_shortcut` 动作描述。按已核对的 V1 输入实现，修饰键顺序为 Ctrl、Alt、Shift、Win，其中前三种使用右侧，Win 使用左侧；不会按名称猜测或统一改成左侧。显式 SendKeys 左右键不由该解析器转换。未知位、鼠标 VK、修饰键作为终键、SendKeys 语法和脚本都不会被当作普通快捷键解析。

源 `DefaultModifiedKeyDownDelay` 为正时保留该延迟；非正值时对 `wps`、`et`、`wpp`、`wpspdf`、`mstsc` 使用 10ms，其他进程为零。转换结果使用版本 13 的 `timing`。缺失或非法设置标记 `source_key_down_delay_unknown`，超过支持上限标记 `unsupported_key_down_delay`，不产生可执行原生动作。按键提交及延迟观测只能证明输入契约，还须在实际目标软件验证业务结果。

触发部分会产生原生应用范围、进程排除名单、专用或保留原功能前缀和键盘重复描述。源应用名单使用版本 15 的显式 `identity:"shell_context"`，区分 Explorer 应用窗口、桌面和任务栏；与旧配置比较时包含匹配域，避免将真实进程范围误认为已迁移。专用键盘前缀使用版本 12 的捕获集合策略，并按源附加键降序、原始列表顺序产生显式 `priority`；保留源规则，不自动扩展配置档别名。保留原功能前缀使用版本 14 的 `retained_prefix`，其中 `capture_scope` 保留引导键黑名单，与动作应用范围独立；Shift/Ctrl 引导键启用主鼠标键阻止捕获。泛型修饰键、长短按分流及鼠标规则仍单独记录缺口；窗口角色投影不证明 UWP/WPS 子进程适配、真实 Hook 交付或动作业务效果。名单有非法格式或有效范围为空时保留待核验状态。

保留原功能策略需要明确的 `PowerKeys_DelayBeforeCtrlKey`、`PowerKeys_ContinuousInputCheckTime`、正值 `PowerKeys_CancelLongPressKeyDelayMs`，并确认 `PowerKeys_IgnoreUnknownKey=false`。提示窗超时须关闭或大于单按取消时限；缺失、非法设置或其他提示窗组合继续标记 `legacy_retained_prefix_timing`，不能用普通前缀超时替代。离线回放只验证显式事件、时间、动作选择和补发序列；实际 Hook 的交付、窗口角色、注入来源与动作效果需要独立验收。

具有同样键位的现有绑定仅记作重叠；动作、应用范围、键盘策略、启用状态和已知条件都一致时才标记配置等价。动作内容相同但缺少专用层或重复策略时，仍需迁移触发语义。配置等价仍需要真实运行验证，整份清单的 `complete` 始终为假。

若专用前缀有一条独立单按规则，且动作仅发送该前缀自身、没有附加行为，工具将其明确折入 `restore_on_tap`，原应用名单保留为 `tap_scope`，保留原行并标记 `prefix_policy_projected_unverified`。恢复超时参考 `InputSettings.PowerKeys_HintWindowDelay`；缺少设置时保留待核验缺口，不猜测数值。进程名单中的完全相同重复项会去重，并记录在 `source_normalizations`；原始规则保留不变。候选触发描述仍须由当前应用校验整体冲突，并验证真实运行结果。

动作转换、配置等价和迁移完成是不同状态。切换前应验证匹配、重复、释放、前缀回退、应用切换及动作实际结果，再处理原触发器的启停，避免两个程序同时认领同一组合。

同一组合按源配置的列表顺序选择首条指定应用规则，再回退到首条全局规则。工具将重叠名单投影为互斥范围，并从全局范围排除所有指定应用；尚未实现的指定应用动作也参与排除，避免误执行全局动作。被前面规则完全遮蔽的条目标记 `shadowed_source_rule`，原行保留。无法确认的进程范围标记 `source_process_precedence_review`。捕获模式保持“先执行动作、未命中再捕获”的顺序，末键兼附加键的关系记为 `source_terminal_preempts_capture`；使用精确模式时仍可能标记 `terminal_prefix_overlap`，不能绕过注册检查。

捕获模式按已捕获的附加键筛选，无附加键的规则可在额外附加键按住时仍匹配，指定应用规则还会优先于全局附加键规则。捕获集合来自实际注册的适用规则，遗漏尚未实现的动作会改变捕获和回退，因此不能把部分候选直接启用为整个专用层。`trigger_gaps` 为空也不证明完整触发等价；启用前须验证额外按键、附加键叠加、默认回退、重复、释放次序及动作效果，不能只验证每条最短组合。

核心目前最多注册 512 个触发器；超出时返回结构化注册错误，保留原配置。提升容量不会改变冲突规则或自动解决应用范围差异。

本地验证工具：

```powershell
python -m unittest discover -s scripts -p test_action_migration.py -v
```
