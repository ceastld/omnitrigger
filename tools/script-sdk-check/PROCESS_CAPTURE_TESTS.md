# 受控进程采集验收

这些测试会实际启动明确的自有夹具和脚本宿主，不使用系统程序替代，不读取个人配置、窗口或剪贴板。仅在Windows显式运行；缺少环境变量或引擎不匹配即失败。

先将 `process-capture-fixture.rs` 编译到全新测试目录，明确设置四个变量：

- `OMNITRIGGER_CAPTURE_FIXTURE`：本源码构建的普通本地EXE绝对路径。
- `OMNITRIGGER_CAPTURE_HOST`：已构建的Boa或QuickJS宿主EXE绝对路径。
- `OMNITRIGGER_CAPTURE_ENGINE`：与宿主能力描述一致的 `boa` 或 `quickjs`。
- `OMNITRIGGER_CAPTURE_CASE_ROOT`：明确新建的普通本地测试父目录。

预热和验收命令从仓库根目录执行：

```powershell
node tools/script-sdk-check/process-capture-test-support.mjs --warmup
node --test --test-concurrency=1 tests/process_capture_parent_kill.test.mjs tests/process_capture_host.test.mjs
```

CI在两种引擎测量构建完成后编译夹具，逐个设置宿主并预热，随后串行执行原生受控ignored用例及上述两份测试。没有环境搜索、缺文件回退或交互桌面回退。

API用例覆盖双流及65536字节边界、超限、非法UTF8、超时、非零默认/显式允许、默认/显式cwd。宿主脚本只返回长度、布尔值和错误代码，不将采集正文放进终态或失败日志。

父进程退出用例通过新目录和随机标记读取本次夹具的PID记录，仅发送信号0观察存活。真正的终止操作只针对测试自己spawn返回的宿主ChildProcess；不会按记录PID终止夹具或其他进程。测试要求Job内两进程在15秒自然期限前退出，不能把宿主已结束当成后代已收尾。失败时只做有界等待，夹具自身有自然退出期限。

这些是实际验收源码；JavaScript语法、静态检查或Fake测试不代表真实Job和宿主结果已经通过。
