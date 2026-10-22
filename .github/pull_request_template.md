## 变更

说明解决的问题、触发条件及修改后的行为。

## 验证

填写实际运行的命令和结果；界面变更可附截图。未运行的检查请说明原因。

- [ ] `cargo fmt --all --check`
- [ ] `cargo test --workspace --locked`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo build -p omnitrigger-app --locked`

## 输入行为

涉及状态机或监听时，列出输入事件序列与预期决策，说明前缀回退、释放、重复、注入及取消/恢复边界的验证情况；不适用时注明。

- [ ] 已同步更新受影响的说明，区分已实现能力与计划。
