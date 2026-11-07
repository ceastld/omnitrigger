# 独立宿主的开发 CLI 升级与恢复

开发 CLI 沿现有安装入口读取调用方提供的制品描述、校验包并执行升级；没有默认发行地址，也不会从本机已有目录或环境变量猜测制品。升级结束返回 `upgraded_not_started`，不准备或启动宿主，不修改绑定，保留当前启用设置。

## 支持范围

此入口只支持已经独立打包、可核验明确数据根 v2 契约的宿主。旧包和新包都必须具有纳入清单哈希校验的 `host-contract.json`，契约标识为 `omnitrigger.legacy.explicit-data-root.v2`。新包还须实现 `status --data-root` 与 `migrate-root` 控制命令及其结构化回执。安装不执行包内脚本来下载其他组件。

旧数据根必须明确指定，具有 `OmniTrigger.LegacyHost.v1` 所有权标记与可核验的数据库。配置同级的目标 `legacy-host-data` 必须尚不存在；源根、目标根和包目录不能重叠。

**不支持原 Quicker profile、原始应用运行目录或已有目标数据根的协调迁移。** 此入口面向独立配置副本，不提供运行中界面会话的配置缓存同步。不扫描个人配置、不猜测旧宿主位置、不合并已有数据库，也不把改名或删除已有根当作升级步骤。普通既有包的无声明安装/登记校验仍保留，但缺少 v2 声明的包不能进入此升级事务。

## 升级描述与命令

沿用 `--install-legacy-package` 的制品描述字段：`archive`、`archive_bytes`、`archive_sha256`、`manifest_sha256`。描述可使用明确的本地归档或已有 HTTPS 传输方式，大小与摘要必须匹配调用方提供的制品。

需要升级时增加 `upgrade` 对象：

| 字段 | 含义 |
| --- | --- |
| `expected_old_manifest` | 已登记旧包的清单 SHA-256 |
| `source_root` | 明确的旧独立数据根绝对路径 |
| `source_database_sha256` | 旧根 `userdata.db` 的 SHA-256 |

下面的 PowerShell 例子使用带空格的绝对路径。先准备已经核验的制品描述，确认其中包含上面的 `upgrade` 对象；旧独立根与配置同级目标根必须是不同位置，目标根须不存在。

```powershell
$legacyApp = 'C:\Apps\Omni Trigger\omnitrigger.exe'
$legacyConfig = 'D:\Trigger Profiles\Work Copy\bindings.json'
$legacyDescriptor = 'D:\Trusted Packages\Legacy Host\upgrade.json'
& $legacyApp --install-legacy-package $legacyDescriptor --config $legacyConfig
if ($LASTEXITCODE -ne 0) { throw '升级被拒绝；读取标准错误中的error.code，先核对状态，不自动重试。' }
```

变量作为独立参数传递，不要把整条命令拼成一个字符串，也不要在变量值内部增加额外引号。JSON里的Windows反斜线须写为 `\\`；使用JSON序列化器时由序列化器处理。

制品描述中的四种 SHA-256（归档、新清单、旧清单、源数据库）均要求 **64位小写十六进制**。PowerShell `Get-FileHash` 的输出须先转换；新清单pin应来自已经核验的制品来源，不能拿归档SHA代替。

```powershell
(Get-FileHash -LiteralPath 'D:\Trusted Packages\Legacy Host\new.zip' -Algorithm SHA256).Hash.ToLowerInvariant()
(Get-FileHash -LiteralPath 'D:\Old Standalone Data\userdata.db' -Algorithm SHA256).Hash.ToLowerInvariant()
```

描述缺失、格式错误或本地归档大小/摘要不符，会在启动安装worker前拒绝。升级缺少 `--config` 的检查在描述前置校验之后；因此同时有坏描述时，先返回描述错误。

升级必须显式提供 `--config`。普通安装入口继续拒绝不同清单的覆盖；只有上述明确的升级描述可进入文件事务。界面安装入口不消费升级描述。

事务先完整获取、验证新包，再以旧程序对明确旧根采集一次状态，要求严格的 `absent` 回执。状态未知、超时或代码映像仍被占用时拒绝，不结束进程、不自动重试。状态采集使用既有程序采集能力的 60000ms 上限，包含首次进程创建，外层期限为 61000ms。同步系统创建调用无法被硬性抢占，实际总耗时仍可能超过期限；返回后继续检查取消和期限，迟到结果不交付。

迁移将旧根复制到全新的目标根，并核验迁移回执；旧根保留。换版时持续持有旧 exe/dll 的保护访问，按清单逐个移动旧文件，随后切入新包并原子发布插件索引。旧包备份、原索引、计划和日志保留。

pending marker在旧宿主状态和索引核验通过之后、迁移之前写入。此后失败会保留marker，普通准备、CLI动作和索引写入会拒绝继续；旧status在写marker前失败则不会新建marker或迁移目标根，但已获取的新stage仍可能保留。首次启动也可能因状态采集时限而拒绝，应用不会自动预热或重试。后续一次显式观察成功不代表首次启动超时已修复；错误本身不证明具体外部原因。

## 显式恢复

发现配置同级 `legacy-upgrade-in-progress.json` 时，先查看marker的stage/根/pin是否属于本次升级，再读取该文件的SHA-256。恢复必须使用明确的配置绝对路径；输入摘要用于确认这一次实际观察的marker，不应自动刷新后盲目重试。

```powershell
$legacyMarker = Join-Path (Split-Path -Parent $legacyConfig) 'legacy-upgrade-in-progress.json'
$legacyMarkerPin = (Get-FileHash -LiteralPath $legacyMarker -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
& $legacyApp --recover-legacy-upgrade $legacyMarkerPin --recovery-mode inspect --config $legacyConfig
if ($LASTEXITCODE -ne 0) { throw '只读检查被拒绝；保留材料并核对error.code。' }
```

以下为各模式的原始参数形式；在PowerShell中可继续使用上面的变量：

```text
omnitrigger.exe --recover-legacy-upgrade <marker摘要> --recovery-mode inspect --config <配置绝对路径>
omnitrigger.exe --recover-legacy-upgrade <marker摘要> --recovery-mode restore-old --config <配置绝对路径>
omnitrigger.exe --recover-legacy-upgrade <marker摘要> --recovery-mode finish-new --config <配置绝对路径>
```

| 模式 | 行为 |
| --- | --- |
| `inspect` | 只读核验固定路径、包闭集、文件哈希、索引与日志；不运行宿主、不创建写锁或日志 |
| `restore-old` | 新包仍在暂存且索引仍旧时，将旧文件分片恢复完整；保留 marker、数据根与证据，返回 `old_package_restored_gate_retained` |
| `finish-new` | 迁移已成立时，继续逐文件换版与索引发布；若新索引已发布，则核验收尾；返回 `new_index_committed_not_started` |

恢复依据固定位置的文件内容判断实际移动结果，不以单条 intent 或 done 推断成功。旧文件必须恰好存在一份且匹配原清单，新完整包必须只位于暂存或最终目录之一。文件缺失、重复、变化，计划外文件，损坏计划、中间日志或其他索引 pin 均拒绝。日志末尾未落完的有界记录保留并报告，不截断它。重新开始文件事务前会归档原计划和日志。

写模式复用同一索引锁，以完整新包分别观察明确旧根和新根的状态，再持续保护实际映像文件。只有索引发布、完整包复核和 marker 归属复验通过后才删除 marker。归属变化或读取失败会返回 `legacy_upgrade_committed_but_blocked`，保留已发布索引及 marker；读取比较再删除不是原子的 compare-and-delete。

`restore-old` 不代表已恢复旧数据根或旧版正常启动；本入口不提供已切入新包或已发布索引后的通用撤回。迁移没有完成、目标根缺失或回执不成立时，`finish-new` 会拒绝。恢复不重做数据库迁移、不重新核验新数据库的全部逻辑行、不修复绝对资源路径，也不提供后台恢复或断电原子保证。


## 先恢复旧文件，再继续新版本

只有 `inspect` 已确认新包在stage、索引仍旧，且原迁移已成立时，下面的两步才构成完整的继续流程。两步都是显式操作：先把旧文件恢复完整并保留门禁，确认marker没有变化，再请求继续新包与索引发布。它们不是恢复旧数据根后重新运行普通安装器。

```powershell
& $legacyApp --recover-legacy-upgrade $legacyMarkerPin --recovery-mode restore-old --config $legacyConfig
if ($LASTEXITCODE -ne 0) { throw '旧文件恢复未确认；不要继续或删除marker。' }
# 成功状态必须是old_package_restored_gate_retained；门禁仍保留。
$legacyMarkerAfter = (Get-FileHash -LiteralPath $legacyMarker -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
if ($legacyMarkerAfter -ne $legacyMarkerPin) { throw 'marker已变化；重新查看归属并执行inspect，不沿用旧授权。' }
& $legacyApp --recover-legacy-upgrade $legacyMarkerPin --recovery-mode finish-new --config $legacyConfig
if ($LASTEXITCODE -ne 0) { throw '新版本收尾未确认；保留marker与恢复材料。' }
# 成功状态为new_index_committed_not_started；这时marker才会清除。
```

`restore-old`后marker原字节保持不变，所以同一摘要可用于下一步；如果文件变化，停止该流程。缺少有效迁移根/回执时，`finish-new`仍会拒绝，不能把删除marker、重新安装或覆盖目标根当作修复。已切入新包或索引已发布时，使用 `finish-new`核验收尾，不先执行 `restore-old`。

## 结构化错误与路径边界

成功在标准输出返回一个JSON对象。拒绝时进程退出码为1，标准错误返回含 `status: "rejected"` 和 `error.code` 的JSON；按 `error.code` 分支，不依赖可能含操作系统文本的 `message`。下面的路径错误假设普通本地路径；云盘、链接、占位或权限错误可能更早被路径门禁拒绝。

| 情况 | `error.code` | 行为边界 |
| --- | --- | --- |
| 描述文件不存在、不是普通文件或JSON无效 | `legacy_install_descriptor_invalid` | 不启动worker，不创建stage |
| 本地归档不存在、不是普通文件、大小/摘要不符；归档或新清单SHA不是小写64位hex | `legacy_install_source_invalid` | 前置拒绝，不覆盖包 |
| 旧清单/源数据库SHA格式不符，或旧根不是绝对路径 | `legacy_upgrade_request_invalid` | 描述校验拒绝 |
| 描述已有效，但升级没有明确 `--config` | `invalid_cli_arguments` | 不启动升级worker |
| 明确旧数据根不存在，或源/目标/包根重叠 | `legacy_upgrade_path_invalid` | 暂存/迁移前拒绝 |
| 目标 `legacy-host-data` 已是普通文件或目录 | `legacy_upgrade_destination_exists` | 不删除、不合并、不当作迁移成功 |
| 没有upgrade对象，却要求替换另一已登记清单 | `legacy_update_requires_installer` | 普通安装入口不覆盖版本 |
| 恢复marker不存在或普通文件无法读取 | `legacy_upgrade_io` | 不猜测stage；写模式在创建索引锁前先读marker |
| marker摘要过期、恢复参数无效、布局/计划/文件存在冲突 | `legacy_upgrade_recovery_conflict` | 不以旧摘要继续；保留现有marker |
| marker仍在时调用普通登记、准备或受门禁保护的CLI动作 | `legacy_upgrade_recovery_required` | 不通过手动删marker绕过；其他设置写入可能由其外层API包装错误 |
| 文件移动目标发生占用或碰撞 | `legacy_upgrade_switch_failed` | 不覆盖目标；按已完成项尝试回退，回退不能确认时返回 `legacy_upgrade_recovery_required` |
| 索引已发布，但尾部journal/marker清理不能确认 | `legacy_upgrade_committed_but_blocked` | 保留新索引及恢复门，不当成旧版本未提交 |

不存在的配置**文件名**不一定单独报错：这些CLI沿配置路径的父目录定位包、插件索引、数据根和marker，并不承诺先读取该bindings文件。应核对目标配置及其所属目录，避免把同目录下的拼写错误误认为应用已识别；恢复的相对配置路径则明确以 `legacy_upgrade_recovery_conflict`拒绝。

marker摘要参数比较不区分十六进制大小写，但制品描述的SHA字段仍要求小写。摘要长度正确却已过期，与marker文件本身不存在是两种不同错误。普通安装不覆盖既有目录；遇到非空/未知材料时保留并拒绝，不提供强制覆盖开关。
