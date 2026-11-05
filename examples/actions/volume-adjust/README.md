# 显式百分点音量调节

Node入口adjust和同步入口adjust-script仅调用同一个ctx.desktop.adjustVolume，不实现音频算法、执行器或媒体键替代。需要应用NativeBridge提供audio.volume_adjust及对应脚本宿主，不要求前台目标、指针或界面。缺少API时先返回runtime_unavailable/not_started，不读取输入、不尝试其他接口。

输入只接受必填deltaPercent，非零整数，范围-100至100个百分点，未知字段拒绝。两个入口保留全部输入交给共享SDK及App校验，不补默认步长或策略。call.json中的+5和call-script.json中的-5只是明确示例值，可修改为其他有效值。

调用会对真实默认多媒体输出端点执行CoreAudio设置。结果保留匿名previousPercent、requestedPercent、currentPercent和保守effectState；读回可能与请求不同，不视为设备确认完成。0/100边界仍沿API合同设置，取消/期限不撤销已发生的变化。没有自动重试、恢复或其他接口回退。

以下CLI模板是硬件动作，会改变真实音量；不由本包校验执行：

```powershell
omnitrigger.exe --config examples/action-packages.json --run-action examples/actions/volume-adjust/call.json
omnitrigger.exe --config examples/action-packages.json --run-action examples/actions/volume-adjust/call-script.json
```

本地作者环境采用sdk声明及严格jsconfig，声明SHA在sdk/snapshot.json。声明不证明宿主已提供API。仓库已有tools/script-sdk-check依赖时，可在仓库根仅做纯检查：

```powershell
node tools/script-sdk-check/node_modules/typescript/bin/tsc --project examples/actions/volume-adjust/jsconfig.json
node --test examples/actions/volume-adjust/recipes.test.mjs
```

RecordingHost只记录共享SDK的请求并返回合成数据，不启动App、网络、音频设备或CLI。single_instance按包和动作ID生效，不承诺Node与同步入口之间的全局互斥。本包不安装或启用触发绑定。
