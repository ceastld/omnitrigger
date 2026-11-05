# 候选动作包快照检查

三个层次分别产出证据：

1. `snapshot_action_candidates.py` 只读候选、既有引用结果和项目动作目录，核对调用哈希/身份/版本/入口，拷贝完整包到全新目录。不解析或执行 JavaScript，不运行应用 CLI。
2. `check_action_modules.mjs` 默认只用 Node 的 `SourceTextModule` 解析静态 import/export-from，检查相对依赖在包内且确实存在。不会 evaluate 模块，也不会调用动作函数；动态导入和函数体业务没有验收。
3. 显式 `--import-trusted-project` 先将完整快照文件哈希与脚本所在项目的 `examples/actions/` 核对，再在子进程导入 Node 入口模块，检查默认导出为函数。该步骤确实执行可信模块的顶层初始化，始终不传 ActionContext、不调用入口函数；不能用于任意用户动作。源码或快照有变化便拒绝。

这些工具独立于应用的 `--check-action`，不会将其静态引用语义改成执行脚本。ActionCall 与插件 ActionSpec 分开记录，插件不被当作动作包安装或执行。

示例（所有路径必须明确，输出目录和报告必须全新）：

```powershell
python -B scripts/snapshot_action_candidates.py --candidate-dir <候选目录> --reference-result <既有引用结果.json> --packages-root examples/actions --out .tmp/candidate-snapshot-NEW
node --experimental-vm-modules scripts/check_action_modules.mjs --snapshot .tmp/candidate-snapshot-NEW/snapshot.json --out .tmp/candidate-snapshot-NEW/static.json
node --experimental-vm-modules scripts/check_action_modules.mjs --snapshot .tmp/candidate-snapshot-NEW/snapshot.json --out .tmp/candidate-snapshot-NEW/trusted-import.json --import-trusted-project
```

快照工具拒绝云盘路径、Offline/Recall 属性、联接点、符号链接、硬链接、多余/越界路径、超限文件和读取期间的身份/内容变化。复制整个动作目录，包括内部模块和说明；不只挑入口文件。每个文件最多 1 MiB，单包最多 512 文件、512 目录和 16 MiB。输出不得位于输入目录中，不覆盖既有证据；完成报告最后写入，失败留下的新目录不能当作成功证据。

模块检查报告只可新建在已验证的快照根目录。包内的百分号编码、外部/内置模块、查询或片段不作为可证明的静态依赖接受。此处是离线完整性检查，并非对并发恶意路径替换的操作系统沙盒，也不是目标软件效果验收。

合成数据测试：

```powershell
python -B -m unittest discover -s scripts -p test_snapshot_action_candidates.py
```

测试用主动抛错的顶层初始化证明静态阶段不执行代码，并复现入口存在而依赖缺失的旧问题。创建符号链接需要系统令牌权限，无法创建时该项单独跳过；reparse/云占位属性的拒绝仍有独立合成测试。
