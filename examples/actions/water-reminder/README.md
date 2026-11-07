# 喝水提醒

每次调用显示一次固定通知：标题“喝水提醒”，正文“整点啦，起来喝杯水吧 💧”，信息级别，右上方。使用已有ui.notify接口和默认显示时长；没有后台循环或自动调度。

选择Node或Script入口之一，输入为空对象`{}`。call.json和call-script.json是同一用途的两种运行时调用例子。两个入口分别为单实例，不应同时绑定到同一触发。安装与调用参见[动作包说明](../../../docs/ACTION_PACKAGES.md)。

成功仅返回`{"submitted":true}`，表示通知请求已接纳，不保证用户已经看到或阅读。无界面、取消、超时或无效回执明确抛错，不自动重试，也不会把未知状态当成显示成功。

本例不读剪贴板、不读写文件、不发送按键或调用外部程序。用内存端口检查两个入口：

```powershell
node --test examples/actions/water-reminder/recipes.test.mjs
```

## 作者工程检查

在本目录运行 `npm ci --ignore-scripts`，再执行 `npm run check`。固定的 TypeScript 5.9.3 严格检查两个入口、共享通知参数与回执处理，以及 Node 默认导出；检查不会调用通知。Node 入口使用异步 ActionContext，Script 入口使用同步 ScriptMain。

本包随附 SDK 声明，配置只加载 ES2022 与显式宿主类型，`types:[]`，不注入 DOM 或 Node 全局。共享 helper 的未知输入、异常及回执由既有运行时检查处理，JSDoc 断言仅连接这些检查和作者类型，不替代输入校验。

仓库根可运行 `node tools/script-sdk-check/check-project-types.mjs examples/actions/water-reminder`。随包声明通过既有 `node tools/script-project.mjs refresh --runtime node --project <本目录>` 显式刷新；入口、通知内容、清单、配置和开发依赖保留。工具仍拒绝覆盖不能识别的作者声明。
