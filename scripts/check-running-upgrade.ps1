# 用于全新 CI 用户或独立 Windows 测试环境。安装器会使用这个用户的安装登记及开始菜单。
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Setup,
    [Parameter(Mandatory)][string]$Archive,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$DataRoot
)
$ErrorActionPreference = 'Stop'
$setupFile = Get-Item -LiteralPath $Setup
$archiveFile = Get-Item -LiteralPath $Archive
$appFile = Get-Item -LiteralPath $Executable
$data = Get-Item -LiteralPath $DataRoot
$probe = New-Item -ItemType Directory -Path (Join-Path ([IO.Path]::GetTempPath()) ("omnitrigger-running-upgrade-" + [guid]::NewGuid().ToString('N')))
$health = Join-Path $probe.FullName 'healthy'
$log = Join-Path $probe.FullName 'install.log'

if (Get-Process -Name $appFile.BaseName -ErrorAction SilentlyContinue | Where-Object Path -EQ $appFile.FullName) {
    throw "覆盖安装测试要求独立的安装目录，当前已有进程在使用 $($appFile.FullName)。"
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class OmniTriggerUpgradeWindow {
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
}
'@

$archiveReader = [IO.Compression.ZipFile]::OpenRead($archiveFile.FullName)
try {
    $payload = $archiveReader.GetEntry('omnitrigger.exe')
    if (-not $payload) { throw '发行 ZIP 缺少主程序。' }
    $stream = $payload.Open()
    try { $expectedHash = (Get-FileHash -InputStream $stream -Algorithm SHA256).Hash }
    finally { $stream.Dispose() }
} finally { $archiveReader.Dispose() }
if ((Get-FileHash -LiteralPath $appFile.FullName -Algorithm SHA256).Hash -ne $expectedHash) {
    throw '待测试的安装程序文件与本次发行 ZIP 不一致。'
}

$start = [Diagnostics.ProcessStartInfo]::new($appFile.FullName)
$start.UseShellExecute = $false
$start.WorkingDirectory = $appFile.DirectoryName
$start.Environment['OMNITRIGGER_HOME'] = $data.FullName
$start.Environment['OMNITRIGGER_UPDATE_HEALTH'] = $health
$ui = [Diagnostics.Process]::Start($start)
try {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        $ui.Refresh()
        if ($ui.HasExited) { throw "安装版未能打开日常窗口，退出码 $($ui.ExitCode)。" }
        $window = $ui.MainWindowHandle
        if ($window -ne [IntPtr]::Zero -and (Test-Path -LiteralPath $health -PathType Leaf)) { break }
        Start-Sleep -Milliseconds 150
    } while ($timer.Elapsed.TotalSeconds -lt 45)
    [uint32]$owner = 0
    $null = [OmniTriggerUpgradeWindow]::GetWindowThreadProcessId($window, [ref]$owner)
    if ($owner -ne $ui.Id -or -not [OmniTriggerUpgradeWindow]::IsWindowVisible($window) -or
        -not (Test-Path -LiteralPath $health -PathType Leaf)) {
        throw '日常窗口或输入监听没有就绪，无法验证真实托盘。'
    }
    if (-not $ui.CloseMainWindow()) { throw '无法向日常窗口发送普通关闭请求。' }
    $timer.Restart()
    while ([OmniTriggerUpgradeWindow]::IsWindowVisible($window) -and $timer.Elapsed.TotalSeconds -lt 10) {
        Start-Sleep -Milliseconds 100
    }
    if ($ui.HasExited -or -not [OmniTriggerUpgradeWindow]::IsWindow($window) -or [OmniTriggerUpgradeWindow]::IsWindowVisible($window)) {
        throw '普通关闭后未保留原进程及隐藏窗口。'
    }

    $install = [Diagnostics.ProcessStartInfo]::new($setupFile.FullName)
    $install.UseShellExecute = $false
    foreach ($argument in @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CLOSEAPPLICATIONS',
        '/NORESTARTAPPLICATIONS', "/DIR=$($appFile.DirectoryName)", "/LOG=$log")) {
        $install.ArgumentList.Add($argument)
    }
    $installer = [Diagnostics.Process]::Start($install)
    try {
        if (-not $installer.WaitForExit(120000)) {
            $installer.Kill($true)
            throw "覆盖安装超时。安装日志：$log"
        }
        if ($installer.ExitCode -ne 0) {
            if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Tail 20 | Write-Host }
            throw "覆盖安装失败，退出码 $($installer.ExitCode)。"
        }
    } finally { $installer.Dispose() }
    if (-not $ui.WaitForExit(10000)) { throw '覆盖安装返回成功，但原托盘进程还在运行。' }
    if ((Get-FileHash -LiteralPath $appFile.FullName -Algorithm SHA256).Hash -ne $expectedHash) {
        throw '覆盖安装后的程序与发行 ZIP 不一致。'
    }
    Write-Host '普通关闭已收起到托盘，覆盖安装成功，原进程已退出。'
} finally {
    # 只收尾本脚本创建并持有句柄的测试进程；上述断言在收尾之前执行。
    if (-not $ui.HasExited) { $ui.Kill(); $null = $ui.WaitForExit(5000) }
    $ui.Dispose()
}
