# 脚本选择示例

将此目录作为动作包放入应用的 `actions` 目录，在动作库设置触发后运行。Node 和轻量脚本展示同样的可搜索选择窗口，示例只返回结果，不读写剪贴板或发送输入。

`ctx.ui.choose({items, title?, placeholder?, timeoutMs?})` 接受 1–200 个候选项，每项包含唯一稳定 `id`、`label` 和可选 `description`。搜索同时匹配这三个字段。↑↓ 移动、Enter 确定，也可点击；Esc 或关闭取消。结果为 `{status:'selected', id}`，或 `{status:'cancelled', reason:'dismissed'|'timeout'}`。默认选择时限为 30 秒，动作总时限仍然生效。

Node 使用 `await`，轻量脚本同步等待。窗口由应用 UI 线程操作，动作工作线程等待，Hook 不等待。独立命令行执行没有 GPUI 界面时返回 `ui_unavailable`；已有选择窗口返回 `ui_busy`。

动作取消、总时限耗尽或宿主退出会请求关闭窗口。后续桌面输入沿用动作提交时捕获的原目标身份。只有选择窗口仍在前台时才尝试恢复原目标；系统拒绝恢复或用户已经切到其他窗口时，后续输入可返回 `target_changed`，不会改选当前前台。

可用 `node --test examples/actions/script-choice/contract.test.mjs` 检查共享契约与 Node 桥接。真实窗口、输入法、滚动和原目标恢复仍需在应用中交互核验。
