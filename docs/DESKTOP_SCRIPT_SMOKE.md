# 桌面脚本的实际效果验收

`desktop_key_smoke` 是显式启用的交互测试，不随普通 `cargo test` 运行。它只创建和操作本轮测试窗口，不读取个人绑定、剪贴板或原目标软件正文。按键观察器只记录本程序注入标记，不记录物理键盘输入。测试取得前台后，若前台变为非测试窗口，会取消输入；释放和清理仍需完成。

准备当前测试程序：

```powershell
cargo test -p omnitrigger-app --bin omnitrigger --no-run --locked
```

由人工或已获授权的窗口控制器激活专用窗口。可设置 `OMNITRIGGER_KEY_SMOKE_READY` 为本地输出文件路径：测试先显示窗口，将本轮 PID 和 HWND 写入该文件，再等待最多 30 秒，等待期间不发送输入。控制器应核对实际窗口身份后激活；超时则失败。未设置此项时沿用直接尝试激活的路径，系统拒绝前台切换时不会继续发送输入。

`OMNITRIGGER_KEY_SMOKE_REPORT` 可指定本地 JSON 结果文件。需要轻量脚本的用例还须设置 `OMNITRIGGER_TEST_SCRIPT_HOST` 为真实宿主程序的完整路径。示例命令：

```powershell
$env:OMNITRIGGER_KEY_SMOKE_READY = 'D:/test-output/key-ready.json'
$env:OMNITRIGGER_KEY_SMOKE_REPORT = 'D:/test-output/key-result.json'
$env:OMNITRIGGER_TEST_SCRIPT_HOST = 'D:/tools/script-host/host.exe'
cargo test -p omnitrigger-app --bin omnitrigger --locked desktop_key_smoke::interactive_desktop_key_sequence_smoke -- --exact --ignored --nocapture --test-threads=1
```

输出目录须已存在，每次使用新的就绪文件，避免误读旧 PID。一次仅运行一个交互测试；测试结束后清除为本轮设置的环境变量。

`interactive_desktop_key_sequence_smoke` 验证原生与 Node 按键时序、取消及失焦收尾；行编辑部分对自建 Win32 多行输入框核验实际文本。设置真实轻量宿主后，还会使用同包的 `recipe-script` 重复行编辑用例，覆盖行内、行尾、行首及 0/10ms 延迟。行首删除可能合并上一行，不能把这条配方描述为删除整个当前行。

`interactive_close_recipe_smoke` 使用真实轻量宿主执行 `common-desktop/call.json`，同时要求动作成功回执及本轮窗口实际销毁。将上述命令的测试名替换为该名称即可运行。此用例验证默认 Alt+F4 分支，不验证其他软件的 Ctrl+W 分支、保存提示或标签页行为。

这些结果只证明自建控件与窗口中的效果，不代表物理触发、原软件快捷键处理、输入法组合态或个人绑定已验收。仅有 `submitted` 回执不能替代窗口或文本状态检查。

调试版还有 `--notification-smoke <空的本地输出目录>`：在独立 GPUI 实例中，经真实 Node 与轻量宿主显示合成通知，检查重叠请求拒绝、到期收尾、活动通知随应用退出清理和前台采样。须先设置 `OMNITRIGGER_TEST_SCRIPT_HOST`；此模式不打开主窗口、不安装 Hook，也不读取个人绑定或剪贴板。第一条通知保留 30 秒，期间将本轮 PID 与通知诊断写入 `ready.json`，供外部窗口工具核对实际可见界面；最终结果写入 `report.json`。前台检查是有界采样，仍需结合非激活窗口属性和实际窗口观察。
