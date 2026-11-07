# 有界窗口调整

该 Node 普通动作最多运行一次 1000ms、32 次矩形提交的调整，使用调用时捕获的原目标。输入必须明确指定 mode（move 或 resize）、handoff 和 restore；示例 call.json 默认不交接或还原窗口。入口为 main.mjs，注册为 single_instance:true。

当指向根窗口与原目标不同，handoff:false 直接返回；true 才请求一次前台交接。只有系统接纳且在预算内确认后，后续窗口操作才使用得到的显式目标租约。默认目标不变，交接拒绝或超时不继续，不重新捕获或回退其他窗口。

最大化窗口在 restore:false 时直接返回；true 时请求一次还原并重读，仍未还原则停止，不轮询。调整只在左键明确按下时进行，按钮未按下或状态不可用便停止。结束也可能是时间或提交预算到达，dragCompleted 始终 false。

窗口操作只确认请求提交，应用和 DPI 变化可能影响结果。effectState:unknown 表示已有请求或旧效果不能确认；错误后不重试、不回滚。该示例不注入、消费或释放输入，不代表完整拖拽或物理输入所有权。

在仓库中可使用 tools/script-sdk-check 的 TypeScript 执行 tsc -p examples/actions/window-adjust/jsconfig.json 检查类型。动作运行不依赖 TypeScript 或其他 npm 包。
