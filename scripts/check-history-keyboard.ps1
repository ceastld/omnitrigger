# 在 Windows 安装包验收桌面运行。按键消息仅投递给本次打开的历史窗口。
param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Scratch
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class HistoryKeyboardCheck {
    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", SetLastError = true)]
    static extern bool PostMessageW(IntPtr window, uint message, UIntPtr key, IntPtr flags);
    public static void Press(IntPtr window, uint key, int scan, bool extended) {
        int flags = 1 | (scan << 16) | (extended ? 1 << 24 : 0);
        if (!PostMessageW(window, 0x0100, (UIntPtr)key, (IntPtr)flags) ||
            !PostMessageW(window, 0x0101, (UIntPtr)key, (IntPtr)(flags | unchecked((int)0xC0000000))))
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
    }
}
'@

function Wait-For([scriptblock]$Condition, [string]$Failure) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        [Windows.Forms.Application]::DoEvents()
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 30
    } while ($timer.Elapsed.TotalSeconds -lt 25)
    throw $Failure
}

$dir = New-Item -ItemType Directory -Path (Join-Path $Scratch ("history-keys-" + [guid]::NewGuid()))
$config = Join-Path $dir.FullName 'bindings.json'
[IO.File]::WriteAllText($config, '{"version":24,"bindings":[]}')
$history = Join-Path $dir.FullName 'clipboard-history.json'
$document = @{
    version = 1; lastId = '2'; revision = '2'
    items = @(
        @{ id = '1'; text = 'oldest-keyboard-fixture'; createdAt = '1' }
        @{ id = '2'; text = 'newest-keyboard-fixture'; createdAt = '2' }
    )
}
[IO.File]::WriteAllText($history, ($document | ConvertTo-Json -Depth 4))
$originalHash = (Get-FileHash -LiteralPath $history).Hash
$form = [Windows.Forms.Form]::new()
$field = [Windows.Forms.TextBox]::new()
$app = $null
try {
    $form.Text = 'OmniTrigger 历史键盘验收输入框'
    $field.Dock = 'Fill'
    $form.Controls.Add($field)
    $form.Show()
    $form.Activate()
    [void]$field.Focus()
    Wait-For { $field.Focused -and [HistoryKeyboardCheck]::GetForegroundWindow() -eq $form.Handle } '专用输入框没有获得前台。'

    $start = [Diagnostics.ProcessStartInfo]::new($Executable)
    $start.UseShellExecute = $false
    foreach ($arg in @('--text-history', '--config', $config)) { $start.ArgumentList.Add($arg) }
    $app = [Diagnostics.Process]::Start($start)
    Wait-For {
        $app.Refresh()
        if ($app.HasExited) { throw "历史窗口提前退出：$($app.ExitCode)" }
        $app.MainWindowTitle -ceq 'OmniTrigger · 独立历史窗口' -and
            [HistoryKeyboardCheck]::GetForegroundWindow() -eq $app.MainWindowHandle
    } '安装版没有打开并激活独立历史窗口。'
    $hwnd = $app.MainWindowHandle
    Wait-For { Test-Path -LiteralPath "$history.lock" -PathType Leaf } '独立窗口未开始读取临时历史。'
    # 允许后台加载两项摘要。End 只移动选择，Enter 只投递一次。
    $loading = [Diagnostics.Stopwatch]::StartNew()
    do {
        [HistoryKeyboardCheck]::Press($hwnd, 0x23, 0x4F, $true)
        [Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 60
    } while ($loading.ElapsedMilliseconds -lt 1800)
    [HistoryKeyboardCheck]::Press($hwnd, 0x0D, 0x1C, $false)
    Wait-For { $field.Text -ceq 'oldest-keyboard-fixture' } '打开后未能直接用 End 和 Enter 将末项送回专用输入框。'
    if ((Get-FileHash -LiteralPath $history).Hash -cne $originalHash) { throw '键盘浏览和发送改变了历史文件。' }
    if (!$app.CloseMainWindow() -or !$app.WaitForExit(20000) -or $app.ExitCode) { throw '独立历史窗口未正常退出。' }
} finally {
    if ($app) {
        if (!$app.HasExited) { $app.Kill(); [void]$app.WaitForExit(10000) }
        $app.Dispose()
    }
    $form.Dispose()
}
