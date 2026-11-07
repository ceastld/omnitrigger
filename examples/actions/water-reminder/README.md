# 喝水提醒

每次调用显示一次固定通知：标题“喝水提醒”，正文“整点啦，起来喝杯水吧 💧”，信息级别，右上方。使用已有ui.notify接口和默认显示时长；没有后台循环或自动调度。

选择Node或Script入口之一，输入为空对象`{}`。call.json和call-script.json是同一用途的两种运行时调用例子。两个入口分别为单实例，不应同时绑定到同一触发。安装与调用参见[动作包说明](../../../docs/ACTION_PACKAGES.md)。

成功仅返回`{"submitted":true}`，表示通知请求已接纳，不保证用户已经看到或阅读。无界面、取消、超时或无效回执明确抛错，不自动重试，也不会把未知状态当成显示成功。

本例不读剪贴板、不读写文件、不发送按键或调用外部程序。用内存端口检查两个入口：

```powershell
node --test examples/actions/water-reminder/recipes.test.mjs
```
