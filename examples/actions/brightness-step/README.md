# 显式策略亮度步进

本包以Node入口`step`和同步脚本入口`step-script`调用同一个`ctx.desktop.stepBrightness`，没有设备算法或独立执行器。需要提供`display.brightness_step`的应用NativeBridge及相应脚本宿主；调用不要求原前台窗口或界面。

输入必须同时包含`direction`和`strategy`，不接受额外字段，不补默认值。direction只能是`up/down`；strategy只能是`physical/wmi_first/physical_then_wmi`。入口原样传递输入，由共享SDK及App宿主校验。

- physical：执行时捕获一次指针，对最近逻辑屏幕下的物理显示器集合逐台步进。
- wmi_first：不读取或依赖指针，直接固定首枚举WMI实例，按支持档位选择，零档和无可用档不写。
- physical_then_wmi：捕获一次指针；仅在确认DDC未写入且无有效成员时尝试首WMI，不在部分或未知写入后补偿另一设备。

`call.json`是明确选择physical增亮的Node ActionCall；`call-script.json`是明确选择wmi_first减亮的同步ActionCall。它们是真实设备调用的输入示例，离线测试不会执行这些硬件操作。包不安装触发绑定或默认启用任何规则。

结果原样保留SDK的匿名逐项回执。外层返回不代表每台设备成功，顶层unknown也不能替代成员confirmed；WMI的set不保证最终读回等于请求。取消或期限不撤销已经发生的写入，不自动重试或反向恢复。single_instance按包和动作ID生效，不承诺两个入口之间的设备互斥。

作者环境使用包内sdk声明和严格jsconfig。在本工程目录中可独立安装并检查，不需要仓库的crates、tools或node_modules：

```powershell
npm ci --ignore-scripts
npm run check
```

TypeScript 5.9.3仅为开发依赖。检查同时覆盖两个入口，保持strict、exactOptionalPropertyTypes、noUncheckedIndexedAccess及skipLibCheck:false；只加载ES2022和本包SDK，不向Script加入DOM或Node全局。入口检查器只分析源码，不导入作者模块。

sdk/snapshot.json记录本包声明的SHA256；check命令逐项核对包内文件，不读取清单中的仓库来源路径。指纹或类型检查通过不替代宿主能力检查。

仓库中的业务回归仍可显式执行：

```powershell
node --test examples/actions/brightness-step/recipes.test.mjs
```

RecordingHost测试仅记录同一共享SDK发出的调用并返回合成回执，不连接应用、网络或设备，不证明硬件效果。
