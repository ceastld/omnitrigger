# 显式策略亮度步进

本包以Node入口`step`和同步脚本入口`step-script`调用同一个`ctx.desktop.stepBrightness`，没有设备算法或独立执行器。需要提供`display.brightness_step`的应用NativeBridge及相应脚本宿主；调用不要求原前台窗口或界面。

输入必须同时包含`direction`和`strategy`，不接受额外字段，不补默认值。direction只能是`up/down`；strategy只能是`physical/wmi_first/physical_then_wmi`。入口原样传递输入，由共享SDK及App宿主校验。

- physical：执行时捕获一次指针，对最近逻辑屏幕下的物理显示器集合逐台步进。
- wmi_first：不读取或依赖指针，直接固定首枚举WMI实例，按支持档位选择，零档和无可用档不写。
- physical_then_wmi：捕获一次指针；仅在确认DDC未写入且无有效成员时尝试首WMI，不在部分或未知写入后补偿另一设备。

`call.json`是明确选择physical增亮的Node ActionCall；`call-script.json`是明确选择wmi_first减亮的同步ActionCall。它们是真实设备调用的输入示例，离线测试不会执行这些硬件操作。包不安装触发绑定或默认启用任何规则。

结果原样保留SDK的匿名逐项回执。外层返回不代表每台设备成功，顶层unknown也不能替代成员confirmed；WMI的set不保证最终读回等于请求。取消或期限不撤销已经发生的写入，不自动重试或反向恢复。single_instance按包和动作ID生效，不承诺两个入口之间的设备互斥。

作者环境使用包内sdk声明和严格jsconfig。sdk/snapshot.json记录声明SHA；声明不替代宿主能力检查。仓库已安装tools/script-sdk-check依赖时，可在仓库根进行纯检查：

```powershell
node tools/script-sdk-check/node_modules/typescript/bin/tsc --project examples/actions/brightness-step/jsconfig.json
node --test examples/actions/brightness-step/recipes.test.mjs
```

RecordingHost测试仅记录同一共享SDK发出的调用并返回合成回执，不连接应用、网络或设备，不证明硬件效果。
