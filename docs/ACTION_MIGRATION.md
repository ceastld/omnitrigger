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

- `source`：含 `PowerKeys` 字典、`MouseActions` 数组及可选 `ExeSettings` 字典的 V1 配置快照。每个前缀包含 `Key`、`IsEnabled`、`KeepOriginKeyFunc`、`BlackList`、`KeyActions`；规则保留原字段。应用配置中的 `MouseActions` 同样纳入。
- `catalog`：完整的 `payload.items`，或直接包含 `items` 的对象。每项含 `id`、`title`、`actionType`。目录的 `hasMore` 必须为假，`totalCount` 必须与条目数一致；分页结果应来自同一个 `snapshotId`。
- `usage`：`Days30` 和 `Days0` 各含 `IsSuccess: true` 与 `Data: { "动作ID": 次数 }`。`Days0` 保留为历史接口原值；工具不推断具体统计起始日期。
- `counters`：已确认的功能计数标识数组，形如 `[{ "Method": "功能名", "Ids": ["计数ID"] }]`。这些 ID 不作为动作排名。工具不猜测未知 ID 的用途。
- `current`：当前 OmniTrigger 的绑定配置，含 `bindings` 数组。

## 筛选及结果含义

筛选所有启用的扩展热键和鼠标规则，再加入近 30 天与历史统计各前 20 项；动作引用按 ID 去重。排名先排除已确认的功能计数，然后取前 20 项。未在本机目录中找到的 ID 仍占据排名，并标记为待解析，不自动替换成下一项。

只有历史计数、没有近期调用且没有启用绑定引用的动作标记为暂缓。直接发送快捷键的规则不依赖动作 ID 的使用计数，不能因缺少计数被判为闲置。所有暂缓项保留原数据，不自动删除。

`operation.native_action` 目前仅转换 V1 `Keystroke` 的 `modifier|VK` 数据，产生独立的 OmniTrigger `send_shortcut` 动作描述。发送时明确使用左侧修饰键。未知位、鼠标 VK、修饰键作为终键、SendKeys 语法和脚本都不会被当作普通快捷键解析。

触发部分单独记录应用范围、进程黑名单、禁用前缀原功能、泛型修饰键、长短按分流和鼠标规则的待核验语义。具有同样键位的现有绑定仅记作重叠；只有动作、触发器、启用状态和已知条件一致时才标记配置等价。配置等价仍需要真实运行验证，整份清单的 `complete` 始终为假。

动作转换、配置等价和迁移完成是不同状态。切换前应验证匹配、重复、释放、前缀回退、应用切换及动作实际结果，再处理原触发器的启停，避免两个程序同时认领同一组合。

核心目前最多注册 512 个触发器；超出时返回结构化注册错误，保留原配置。提升容量不会改变冲突规则或自动解决应用范围差异。

本地验证工具：

```powershell
python -m unittest discover -s scripts -p test_action_migration.py -v
```
