# 窗口与指针接口

Node 入口逐次 await，Script 入口同步返回。接口运行在动作 worker，复用原目标身份、前台、会话、输入桌面、取消和期限守卫，不进入 Hook 同步路径。接口存在不表示本次调用具备 Native 上下文；只有作用域服务时，窗口接口返回 native_bridge_unavailable。

| SDK 方法 | 行为 |
| --- | --- |
| getWindowGeometry() | 读取原目标外框、DWM 可见框、显示器/工作区、主屏物理尺寸、窗口 DPI 和最大化状态。 |
| setWindowRect(rect,{frame}) | 显式按 outer 或 visible 坐标提交矩形，不隐式还原、激活或裁剪工作区。 |
| getPointerState() | 调用时采样物理屏幕点与当前三键状态。未按下与不可用合并为 not_down_or_unavailable，来源不可区分。 |
| getPointingRoot() | 调用时只读观察指向子窗、根窗身份和物理坐标；观察本身不是操作目标租约。 |
| handoffPointingRoot(expected,options) | 显式请求一次前台交接；只有接纳且在预算内确认才返回现有 CapturedTarget。 |
| trackWindowDrag(options) | 单次有界移动或调整大小，返回停止原因及计数，不代表完整拖拽。 |

矩形坐标为 i32，写入宽高为 1～65535，右/下边界不得溢出。DPI 是系统读取值，不是坐标缩放因子；接口使用物理屏幕坐标。主屏尺寸与目标所在显示器工作区分开提供，不以最近显示器代替主屏。DWM 可见边界不可用时明确失败。

handoffPointingRoot 必须提供完整观察及 validForMs（2～5000）和 confirmTimeoutMs（1～1000，严格小于有效期）。requested 只表示请求已发生；requestAccepted 或 foregroundConfirmed 未满足时没有新租约。拒绝/超时不重试，不把此观察自动用于输入。成功的引用只属于当前调用，每次操作重验完整身份、活动租约、单调有效期、前台、取消及调用期限。新的有效捕获尝试会使旧引用失效。

显式目标方法为 getWindowGeometryFor、setWindowRectFor、restoreWindowFor、toggleMaximizeWindowFor 和 trackWindowDragFor。它们复用同一 CapturedTarget 格式，默认目标始终不变；引用过期或身份改变时失败，不重捕获、不回退默认窗口。restore 请求正常显示，toggleMaximize 按当前最大化状态切换，两者不混用。windowAction 另支持原目标的 restore；既有其他操作保持。

有界调整参数包含 mode、button、maxDurationMs（20～2000）和 maxUpdates（1～64）。最多 201 次主采样，10ms 协作等待；当前按钮必须明确按下，提交前再次采样确认。resize 在任一轴累积位移达到 10 像素后使用两轴增量。可显式提供 move 的 snap 物理边界、距离及等待参数；没有无限循环或自动延长预算。接口不注入、消费或释放按钮，不能据此取得物理输入所有权。

写入、还原与切换只确认请求提交，应用和 DPI 消息可能调整最终结果。发起后的取消、超时或失败为 unknown 时，不能声称没有效果，也不自动重试或补偿。读取失败为 not_started；累计动作仍须保留之前步骤已发生的效果。有界停止回执始终 dragCompleted:false。

示例见[有界窗口调整](../examples/actions/window-adjust/README.md)。该示例只执行一次有界调整，并要求显式选择是否交接或还原。
