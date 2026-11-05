# App 中的 Node 输出采集验收

此工具实际调用指定 App 的 `--run-action`，只使用本次新建的合成动作目录。它不启动 GUI、Hook 或剪贴板采集，也不修改个人配置。导入工具模块或运行纯 Fake 测试不会启动任何子进程。

先构建 App 和已有的 `process-capture-fixture.rs`，再在 Windows 上显式传入三份自有 EXE 的普通本地绝对路径，以及不存在的新目录。测试父目录必须已经存在：

```powershell
node tools/script-sdk-check/check-app-node-capture.mjs --app "D:\Build\omnitrigger.exe" --node "C:\Program Files\nodejs\node.exe" --fixture "D:\CaptureChecks\fixture.exe" --case-root "D:\CaptureChecks\node-app-001" --cold-timeout-ms 60000 --case-timeout-ms 20000 --observe-ms 3000
```

不从 PATH 寻找 EXE，不回退到其他宿主，不覆盖或删除旧目录。路径先经过已有的云盘、占位属性、链接和普通文件检查。工具将明确指定的 Node 传给 App，并用作者入口里的 `process.execPath` 核实；测试夹具先执行 `warmup` 校验固定签名。App 的第一次合成采集另用冷启动预算预热，后续才测试短超时。

默认 `--suite all` 包含12个独立用例：预热、双流合计65536字节、共享容量溢出、非法UTF-8、非零码默认拒绝及显式允许、默认及显式工作目录、采集自身超时、合成Node宿主取消帧、未 `await`、Node自身异常退出。每项只执行一次；错误或未知立即结束该批，不重试动作。

`--suite normal` 仅运行前9个非控制用例；`--suite control` 只运行3个控制用例。选择必须显式给出，失败不会自动切换或降级；normal通过不证明取消或异常退出的收尾。

取消帧用例由合成作者向自己的 Node 宿主 stdin 事件注入已有的 `cancel` 帧，验证宿主取消和断连路径；它不证明 ActionService 用户取消或外层60秒期限。回执明确保留 `actionServiceUserCancelVerified:false` 和 `outerActionDeadlineVerified:false`。

控制用例按各自契约匹配精确终态：取消帧要求 App 的 `status:"cancelled"` 和错误码 `cancelled`；未 `await` 与自身异常退出要求 `status:"failed"`，错误码分别为 `pending_process_capture/runtime_exit`。不会同时接受多个状态来跳过错误；存活与清理观察条件仍须独立满足。

控制用例先启动已有夹具的 `descendant` 模式。读取并校验同 token 的新鲜 PID 记录后，工具确认本次 App、Node 和两个夹具确实存活，才放行合成作者。Node强杀仅调用自身 PID；记录中的夹具 PID 只用于 signal 0 观察，不用于终止进程。

协作清理的成功条件是：Node与两个夹具在观察窗口内消失，且 App 在该次存活观察前后都仍然存活。若 App 已退出、观察不确定，或只等到夹具自然期限，不记为通过。夹具记录在放行前必须小于5秒，观察窗口最多5秒，早于夹具15秒自然退出期限。App CLI是短命进程，若未捕获到这段存活窗口，结果为未证明，不能用App退出时OS关闭Job的结果补充为成功。

这与轻量宿主退出时由OS关闭其自身Job句柄不同。Node桥接的Job由App持有，验收的是连接丢失检测、协作取消和清理。已有Boa/QuickJS强杀验收应继续独立运行。

成功退出0，失败退出1。标准输出与新目录中的 `receipt.json` 只包含代码、布尔值、长度和观察耗时，不保存采集正文。App错误消息和原始stdout/stderr不回显。失败时工具可停止自己 `spawn` 返回的App句柄以有界收尾；不会根据记录PID终止其他进程，也不会删除证据目录。

已取得完整终态时，`terminal` 仅保存 `status/code/effect/method` 四个元数据字段，不保存 `message` 或 `output`。无法取得或解析终态时保留 `null`；不能用一次失败后的强制收尾补成成功证据。

短命CLI无法提供足够的存活观察窗口时，可单独运行App内部原生验收：显式设置 `OMNITRIGGER_APP_CAPTURE_NODE`、`OMNITRIGGER_CAPTURE_FIXTURE` 和已存在的 `OMNITRIGGER_APP_CAPTURE_CASE_ROOT`，然后执行：

```powershell
cargo test -p omnitrigger-app --bin omnitrigger process_capture_host::node_native::node_self_exit_cleans_capture_job_while_parent_test_process_lives -- --ignored --exact --test-threads=1
```

该用例组合生产 `NodeRuntime`、桥接和采集适配器，父测试进程自然存活；放行前保留自有Node、夹具及后代的只读进程句柄，在3秒内观察退出，早于15秒自然期限。它只证明Node自身异常退出后的协作清理，不证明ActionService用户取消或外层期限，也不代替CLI控制组的失败记录。

相同环境下，测试 `process_capture_host::node_native::action_service_cancel_cleans_capture_job_while_parent_test_process_lives` 验证 `ActionService.cancel_id`：通过真实动作包提交后再取消，分别核对采集实际返回的取消错误、服务的 Started/Cancelled 终态、占用释放及三个进程退出。作者不注入取消帧、不自行退出；该用例不操作物理界面，也不证明外层期限。

纯 Fake 验证命令：

```powershell
node --test tests/app_node_capture_check.test.mjs
```

Fake通过只证明验收工具的判定和控制逻辑；实际 App、Node、夹具与Job效果必须执行上面的显式真实命令后判断。
