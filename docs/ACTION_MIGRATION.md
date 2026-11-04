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

可选的 `--mouse-settings <文件>` 提供全局鼠标设置，也可放在 `source.MouseSettings` 中。支持直接对象、成功的 `resultJson` 导出响应及含 `Values:[{Name,Value}]` 的快照；重复名称或失败响应会被拒绝。需包含 `SpecialExeList`、`DisableOnFullscreenApp`、`EnableReverseVScroll`，缺失时保留源鼠标行并标记缺口。

目前可为无位置、无去抖、无需激活窗口的侧键 1／2 加滚轮或鼠标按下规则生成触发描述：滚轮使用逐事件策略，进程条件放入 `pointer_scope`。白名单非空时保留其优先于黑名单的语义，其他情况合并全局和本条排除；不能表达的路径名单不会被丢弃后继续转换。列表记录当前配置中同组合的编号，只有通知或不同动作时不会标成已迁移。前缀生命周期、额外鼠标按住状态、队列调度和窗口身份差异继续作为显式缺口；生成触发描述不代表可以直接启用。

全局鼠标行的 `source_requirements` 分别记录控制键与区域含义。无控制键、通用 Ctrl／Shift／Alt、需要捕获所有权的普通键、明确左右侧的修饰键不再混为同一种缺口。明确的左 Alt（VK 164）加鼠标规则可生成候选描述，保留修饰键按下交付、异步状态采样、额外按住键和队列差异；不会将通用 Ctrl 擅自改成左 Ctrl。

完整上、下、左、右边框会记录为 1px 的源区域需求，但暂不自动转换边缘规则。无控制键时的排除条件、相邻边角的列表优先顺序，以及多屏接缝策略仍待核实。识别出了区域枚举不代表触发语义已经等价，也不以本项目默认的 8px 边缘替换源范围。

## 输入格式

- `source`：含 `PowerKeys` 字典、`MouseActions` 数组及可选 `ExeSettings` 字典的 V1 配置快照。每个前缀包含 `Key`、`IsEnabled`、`KeepOriginKeyFunc`、`BlackList`、`KeyActions`；规则保留原字段。应用配置中的 `MouseActions` 同样纳入。只读导出的 `InputSettings` 用于确认单按恢复超时及 `DefaultModifiedKeyDownDelay`；缺少后者时不能确认快捷键的输入时序。
- `catalog`：完整的 `payload.items`，或直接包含 `items` 的对象。每项含 `id`、`title`、`actionType`。目录的 `hasMore` 必须为假，`totalCount` 必须与条目数一致；分页结果应来自同一个 `snapshotId`。
- `usage`：`Days30` 和 `Days0` 各含 `IsSuccess: true` 与 `Data: { "动作ID": 次数 }`。`Days0` 保留为历史接口原值；工具不推断具体统计起始日期。
- `counters`：已确认的功能计数标识数组，形如 `[{ "Method": "功能名", "Ids": ["计数ID"] }]`。这些 ID 不作为动作排名。工具不猜测未知 ID 的用途。
- `current`：当前 OmniTrigger 的绑定配置，含 `bindings` 数组。

可选 `--bodies .tmp/migration/bodies.json` 提供动作正文记录数组，或 `{ "Items": [...] }`；也接受成功导出响应的 `resultJson` 包装。每项使用 `Id`、`ActionType`、`UseTemplate`、`Resolution`、`StoredData`、`EffectiveData`。身份必须唯一；引用缺失仍保留原行。当前只投影 `ActionType:"SendKeys"`、`UseTemplate:false`、`Resolution:"local"` 且两个正文值完全相同的记录；模板及来源未核验的正文继续标记缺口，不用旧正文代替模板，也不将空正文当作已迁移。

这种基本键盘动作的逐行格式与快速操作类型 2 的 WinForms SendKeys 不同。支持 `@修饰键+普通键`（或 `|` 分隔，逗号表示多键）、普通键的 `!键名`、`;毫秒` 和字面文本行／`%文字`。一行中的普通键共享修饰键；泛型 Ctrl/Shift/Alt 按源规则使用右侧，显式左右键保持不变。非等待行之后、下一行之前自动加入 30ms，即使下一行本身也是等待：Shift+Home、`;50`、Backspace 的配置等待合计 80ms。负数／零等待不执行睡眠，但仍取消下一行的自动等待。未知键、独立修饰键轻点、预算超限或特殊 Unicode 标记不会被跳过为无操作。输出调用[键盘配方示例](../examples/actions/keyboard-sequence/recipe.mjs)，运行时只依赖 OmniTrigger 的桌面 API。

## 筛选及结果含义

筛选所有启用的扩展热键和鼠标规则，再加入近 30 天与历史统计各前 20 项；动作引用按 ID 去重。排名先排除已确认的功能计数，然后取前 20 项。未在本机目录中找到的 ID 仍占据排名，并标记为待解析，不自动替换成下一项。

只有历史计数、没有近期调用且没有启用绑定引用的动作标记为暂缓。直接发送快捷键的规则不依赖动作 ID 的使用计数，不能因缺少计数被判为闲置。所有暂缓项保留原数据，不自动删除。

`operation.native_action` 可转换 V1 `Keystroke` 的 `modifier|VK` 数据，产生独立的 OmniTrigger `send_shortcut` 动作描述。按已核对的 V1 输入实现，修饰键顺序为 Ctrl、Alt、Shift、Win，其中前三种使用右侧，Win 使用左侧；不会按名称猜测或统一改成左侧。显式 SendKeys 左右键不由该解析器转换。未知位、鼠标 VK、修饰键作为终键、SendKeys 语法和脚本都不会被当作普通快捷键解析。

内置亮度加减可复用现有 `adjust_system`，候选步长分别为 `5`、`-5` 个百分点。目标屏幕采样时刻和硬件量化仍保留在 `operation.action_gaps`，不因动作描述相同就标为配置等价。汇总的 `native_action_requires_semantics_review` 单独计数这些已有候选但仍需核对的动作；它们也包含在 `native_action_available` 中。音量 Shell 命令不直接替换成媒体键或固定百分点，避免悄悄改变分发路径和步长。

V1 `InputText` 的非空字面文本转换成现有 `local` 动作的一步 `input_text`，通过原目标窗口直接发送 Unicode，不使用剪贴板或附加回车。保存时将 `$` 转为 `$$`，以保留 `${…}` 等字面内容；原规则数据保持不变，停用示例见 [`literal-text.json`](../examples/literal-text.json)。动态剪贴板／选区／日期占位符及 `FILE:` 内容读取标记 `source_text_expansion_required`，不静默替换。源 LF／CRLF 使用物理 Return，与当前文本 API 的 Unicode CR 不同，继续标记 `source_text_newline_key_semantics`。NUL、无效代理项、超出 UTF-16 或本地动作配置上限的文本不会产生可执行描述；附加通知仍保留行为缺口。生成结果须在实际目标窗口验证，源类型不会仅因已有文本 API 就被标记为完成。

Windows 可运行 `cargo test -p omnitrigger-app interactive_local_literal_text_smoke --locked -- --ignored --nocapture`，在自建输入框中经真实本地执行器验证字面文本和选区替换。测试暂时取得该窗口焦点，核对实际文字及剪贴板序号，不读取剪贴板内容；这项观察不代替原目标软件和触发器验收。

源对 U+E000–U+E0FF 另加扩展键标记，与当前 Unicode 输入标记不同，这类字符保留 `source_text_extended_flag_semantics` 缺口。

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
