# 发送按键与快捷键

在触发器详情中选择“发送快捷键”，填写单个按键或修饰键组合，保存后切换到目标窗口测试。

| 输入示例 | 发送内容 |
|---|---|
| `Backspace` | 退格 |
| `Left` / `Up` / `Right` / `Down` | 方向键 |
| `Ctrl+C` | 复制快捷键 |
| `Ctrl+Shift+Left` | 按词向左选择 |
| `RightCtrl+Home` | 右 Ctrl 与 Home |
| `VolumeUp` / `音量增加` | 系统音量增加，显示 Windows 原生浮层 |
| `VolumeDown` / `音量减少` | 系统音量减少，显示 Windows 原生浮层 |
| `VolumeMute` / `静音` | 切换系统静音 |

支持字母、数字、F1–F24、导航键、常用编辑键、小键盘及系统音量键。`Ctrl`、`Shift`、`Alt`、`Win` 默认表示左侧，可明确填写 `RightCtrl` 等右侧名称。最多四种修饰键加一个普通键；重复修饰键、纯修饰键或多个普通键会报错，不自动改写。

普通按键每次触发发送一次完整按下与释放；配置 `keyboard.repeat` 后可由系统键盘重复再次触发。默认零延迟的独立 `VolumeUp`、`VolumeDown`、`VolumeMute` 每次向 Windows Shell 提交一个对应媒体命令，避免被前台程序吞掉；调节步长、默认输出和原生浮层位置由 Windows 决定，不叠加本软件的音量浮层。带修饰键或显式正延迟的组合走按键输入路径。按键动作有独立后台队列，最多四个待处理或未读结果，与外部程序执行分开。队列满、权限隔离或输入失败会在动作状态中报错，不自动重试。

提交时记录前台窗口及进程身份，执行时核验原目标，不主动切换焦点。已按住且属于目标快捷键的修饰键只借用，不补发释放；存在其他已按住的修饰键或目标键时取消。自身注入带专用标记，不再次触发组合或进入录制。其他程序的 Hook 仍可能拦截输入，部分成功后的释放收尾也不能保证无损恢复。

鼠标终键或滚轮绑定的快捷键支持按每次调用取消：暂停监听、开始录制、替换绑定或收到 `Esc` 通知时，停止这些调用的后续输入。取消请求保留队列占用，直到对应发送结果被读取；已发送的输入不会撤销，释放仍遵循上述按键所有权规则。

当前绑定配置保存为版本 23，读取兼容版本 1–23。版本 5 引入的统一动作字段继续沿用，例如：

```json
"action": { "kind": "send_shortcut", "keys": [162, 67] }
```

`keys` 依次为 Windows 左右明确的虚拟键码，最后一个是目标键；上例为左 Ctrl（162）与 C（67）。旧程序不认识该动作会明确拒绝读取，回退前请恢复配置备份。

版本 13 可添加按下延迟和进程覆盖，例如：

```json
"action": {
  "kind": "send_shortcut",
  "keys": [163, 67],
  "timing": {
    "key_down_delay_ms": 0,
    "process_overrides": {"wps": 10, "mstsc": 10}
  }
}
```

延迟为正时，先按下本次负责的修饰键，等待指定时间，再按下普通键、等待、释放普通键，最后逆序释放本次修饰键。零延迟使用一次输入提交。等待期间持续检查退出取消和原目标；失败后只尝试释放本次已提交且尚未释放的按键，不重发动作。进程覆盖的格式和范围见[配置说明](CONFIGURATION.md#版本与备份)。界面编辑键位会保留时序；时序目前通过配置文件设置。

脚本可用 `ctx.desktop.sendShortcut(keys, {keyDownDelayMs})` 或 `sendKeySequence(modifiers, keys, {keyDownDelayMs})`。后者在整组普通键期间保持修饰键按下，适合双击同一组合或编辑器和弦；多个独立快捷键则逐个 `await sendShortcut`，步骤间可调用 `ctx.delay`。见[键盘序列示例](../examples/actions/keyboard-sequence/)及[脚本 API](ACTION_PACKAGES.md)。

[CapsLock 导航示例](../examples/capslock-navigation.json) 配置了 `CapsLock+O → Backspace`，`I/J/K/L → ↑/←/↓/→`。可独立试用：

```powershell
.\target\debug\omnitrigger.exe --config .\examples\capslock-navigation.json
```

程序编辑会保存到指定文件；需保留原示例时，先复制到 `.local`。上述示例的单独 CapsLock 仍遵循前缀回放规则。应用范围、专用层、单按恢复及重复策略需显式配置，见[配置说明](CONFIGURATION.md#专用键盘层与按住重复)。
