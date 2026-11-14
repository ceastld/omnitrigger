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
using System.Text;
public static class OmniTriggerUpgradeWindow {
    private delegate bool EnumWindow(IntPtr hwnd, IntPtr parameter);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindow callback, IntPtr parameter);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr hwnd, StringBuilder title, int capacity);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, UIntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern uint RegisterWindowMessage(string message);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint message, UIntPtr wParam, IntPtr lParam, uint flags, uint timeout, out UIntPtr result);
    public static IntPtr FindAppWindow(uint processId, string expectedTitle) {
        IntPtr workspace = IntPtr.Zero;
        EnumWindows((hwnd, unused) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != processId || !IsWindowVisible(hwnd)) return true;
            var title = new StringBuilder(256);
            GetWindowText(hwnd, title, title.Capacity);
            string name = title.ToString();
            if (!string.IsNullOrEmpty(expectedTitle)) {
                if (name != expectedTitle) return true;
            } else if (name != "OmniTrigger" && !name.StartsWith("OmniTrigger — 测试：", StringComparison.Ordinal)) return true;
            workspace = hwnd;
            return false;
        }, IntPtr.Zero);
        return workspace;
    }
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

function New-AppStart {
    $start = [Diagnostics.ProcessStartInfo]::new($appFile.FullName)
    $start.UseShellExecute = $false
    $start.WorkingDirectory = $appFile.DirectoryName
    $start.Environment['OMNITRIGGER_HOME'] = $data.FullName
    $null = $start.Environment.Remove('OMNITRIGGER_UPDATE_HEALTH')
    $null = $start.Environment.Remove('OMNITRIGGER_UPDATE_ERROR')
    return $start
}

function Wait-AppWindow([Diagnostics.Process]$ui, [string]$healthFile, [string]$expectedTitle = '') {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        $ui.Refresh()
        if ($ui.HasExited) { throw "安装版未能打开窗口，退出码 $($ui.ExitCode)。" }
        $window = [OmniTriggerUpgradeWindow]::FindAppWindow($ui.Id, $expectedTitle)
        if ($window -ne [IntPtr]::Zero -and [OmniTriggerUpgradeWindow]::IsWindowVisible($window) -and
            (-not $healthFile -or (Test-Path -LiteralPath $healthFile -PathType Leaf))) { break }
        Start-Sleep -Milliseconds 150
    } while ($timer.Elapsed.TotalSeconds -lt 90)
    [uint32]$owner = 0
    $null = [OmniTriggerUpgradeWindow]::GetWindowThreadProcessId($window, [ref]$owner)
    if ($owner -ne $ui.Id -or -not [OmniTriggerUpgradeWindow]::IsWindowVisible($window) -or
        ($healthFile -and -not (Test-Path -LiteralPath $healthFile -PathType Leaf))) {
        throw "安装版窗口或预期的健康回执没有就绪。期望窗口标题为 '$expectedTitle'。"
    }
    return $window
}

function Close-ToTray([Diagnostics.Process]$ui, [string]$healthFile) {
    $window = Wait-AppWindow $ui $healthFile
    $taskbarCreated = [OmniTriggerUpgradeWindow]::RegisterWindowMessage('TaskbarCreated')
    [UIntPtr]$result = [UIntPtr]::Zero
    if ($taskbarCreated -eq 0 -or [OmniTriggerUpgradeWindow]::SendMessageTimeout(
        $window, $taskbarCreated, [UIntPtr]::Zero, [IntPtr]::Zero, 2, 5000, [ref]$result) -eq [IntPtr]::Zero) {
        throw '测试窗口未处理任务栏重建通知。'
    }
    if (-not $ui.CloseMainWindow()) { throw '无法向日常窗口发送普通关闭请求。' }
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ([OmniTriggerUpgradeWindow]::IsWindowVisible($window) -and $timer.Elapsed.TotalSeconds -lt 10) {
        Start-Sleep -Milliseconds 100
    }
    if ($ui.HasExited -or -not [OmniTriggerUpgradeWindow]::IsWindow($window) -or [OmniTriggerUpgradeWindow]::IsWindowVisible($window)) {
        throw '普通关闭后未保留原进程及隐藏窗口。'
    }
    if ([OmniTriggerUpgradeWindow]::SendMessageTimeout(
        $window, $taskbarCreated, [UIntPtr]::Zero, [IntPtr]::Zero, 2, 5000, [ref]$result) -eq [IntPtr]::Zero -or
        [OmniTriggerUpgradeWindow]::IsWindowVisible($window) -or $ui.HasExited) {
        throw '任务栏重建通知使托盘中的日常窗口弹出或退出。'
    }
    return $window
}

function Stop-TestProcess([Diagnostics.Process]$ui) {
    try {
        if (-not $ui.HasExited) { $ui.Kill(); $null = $ui.WaitForExit(5000) }
    } finally { $ui.Dispose() }
}

# 为 CI 安装版构造独立的默认目录，使外部副本提示走正常启动入口。
$local = Join-Path $probe.FullName 'ignore-copy'
$daily = Join-Path $local 'OmniTrigger'
$external = Join-Path $local 'Packages\Example\LocalCache\Local\OmniTrigger'
$device = Join-Path $daily 'device'
$null = New-Item -ItemType Directory -Force -Path $device, $external
$sourceFile = Join-Path $external 'bindings.json'
$ignoredFile = Join-Path $device 'external-copies.json'
[IO.File]::WriteAllText((Join-Path $daily 'bindings.json'), '{"version":1,"bindings":[]}')
[IO.File]::WriteAllText($sourceFile, '{"version":1,"bindings":[]}')
[IO.File]::SetLastWriteTimeUtc($sourceFile, [DateTime]::UtcNow.AddHours(1))
[IO.File]::WriteAllText($ignoredFile, '{"prior":100}')
$ignoreStart = New-AppStart
$null = $ignoreStart.Environment.Remove('OMNITRIGGER_HOME')
$ignoreStart.Environment['LOCALAPPDATA'] = $local
foreach ($case in '文件占用', '解除占用', '同一副本', '副本有新修改') {
    if ($case -eq '副本有新修改') {
        [IO.File]::SetLastWriteTimeUtc($sourceFile, [IO.File]::GetLastWriteTimeUtc($sourceFile).AddHours(1))
    }
    $held = if ($case -eq '文件占用') { [IO.File]::Open($ignoredFile, 'Open', 'Read', 'Read') }
    try {
        $ui = [Diagnostics.Process]::Start($ignoreStart)
        try {
            if ($case -ne '同一副本') {
                $prompt = Wait-AppWindow $ui '' 'OmniTrigger 发现外部数据副本'
                if ([OmniTriggerUpgradeWindow]::GetDlgItem($prompt, 7) -eq [IntPtr]::Zero -or
                    -not [OmniTriggerUpgradeWindow]::PostMessage($prompt, 0x0111, [UIntPtr]7, [IntPtr]::Zero)) {
                    throw "未能在副本提示中选择忽略（$case）。"
                }
            }
            $null = Wait-AppWindow $ui '' 'OmniTrigger'
            $saved = Get-Content -LiteralPath $ignoredFile -Raw | ConvertFrom-Json -AsHashtable
            if ($saved.prior -ne 100) { throw "旧的忽略记录丢失（$case）。" }
            if ($case -eq '文件占用') {
                if ($saved.Count -ne 1) { throw '被占用的忽略记录发生变化。' }
            } else {
                $expected = ([DateTimeOffset][IO.File]::GetLastWriteTimeUtc($sourceFile)).ToUnixTimeMilliseconds()
                if ($saved[$external.ToLowerInvariant()] -ne $expected) { throw "未保存对应的副本版本（$case）。" }
            }
        } finally { Stop-TestProcess $ui }
    } finally { if ($held) { $held.Dispose() } }
}
Write-Host '已验证忽略选择在文件占用后可保存，重复启动跳过旧副本，副本更新后重新提示。'

# 用户选择导入后，编辑器占用应直接进入日常窗口。释放后重新启动可正常导入。
$importDaily = Join-Path $daily 'bindings.json'
[IO.File]::WriteAllText($sourceFile, '{"version":1, "bindings":[]}')
[IO.File]::SetLastWriteTimeUtc($sourceFile, [DateTime]::UtcNow.AddHours(4))
$originalHash = (Get-FileHash -LiteralPath $importDaily).Hash
$sourceHash = (Get-FileHash -LiteralPath $sourceFile).Hash
foreach ($busy in @($true, $false)) {
    $writer = $null
    try {
        if ($busy) {
            $writer = [IO.File]::Open((Join-Path $daily 'wheels.json.lock'), 'OpenOrCreate', 'ReadWrite', 'ReadWrite')
            $writer.Lock(0, 1)
        }
        $ui = [Diagnostics.Process]::Start($ignoreStart)
        try {
            $prompt = Wait-AppWindow $ui '' 'OmniTrigger 发现外部数据副本'
            if ([OmniTriggerUpgradeWindow]::GetDlgItem($prompt, 6) -eq [IntPtr]::Zero -or
                -not [OmniTriggerUpgradeWindow]::PostMessage($prompt, 0x0111, [UIntPtr]6, [IntPtr]::Zero)) {
                throw '未能在外部副本提示中选择导入。'
            }
            $null = Wait-AppWindow $ui '' 'OmniTrigger'
            $expectedConfigHash = if ($busy) { $originalHash } else { $sourceHash }
            if ((Get-FileHash -LiteralPath $importDaily).Hash -ne $expectedConfigHash) { throw '日常配置与导入结果不符。' }
        } finally { Stop-TestProcess $ui }
    } finally { if ($writer) { $writer.Dispose() } }
}
Write-Host '已验证配置被编辑器占用时直接打开日常窗口，释放后可重新导入。'

# 使用单独的数据根，验证安装版窗口确实持有测试档，正常退出后才允许重置。
$profileRoot = Join-Path $probe.FullName 'profiles-test'
$testStart = New-AppStart
foreach ($arg in @('--data-root', $profileRoot, '--test-workspace')) { $testStart.ArgumentList.Add($arg) }
$testUi = [Diagnostics.Process]::Start($testStart)
try {
    $null = Wait-AppWindow $testUi ''
    $resetArgs = @('--data-root', $profileRoot, '--profile-reset', 'testing')
    $resetError = Join-Path $probe.FullName 'reset-error.json'
    # 重置被占用的配置档应以非零退出码和结构化错误返回。
    $reset = New-AppStart
    foreach ($arg in $resetArgs) { $reset.ArgumentList.Add($arg) }
    $reset.ArgumentList.Add('--relaunch-result')
    $reset.ArgumentList.Add($resetError)
    $attempt = [Diagnostics.Process]::Start($reset)
    try {
        if (-not $attempt.WaitForExit(20000) -or $attempt.ExitCode -ne 1) { throw '测试窗口打开时未拒绝重置。' }
    } finally { Stop-TestProcess $attempt }
    $receipt = Get-Content -Raw -LiteralPath $resetError | ConvertFrom-Json
    if ($receipt.ok -or ($receipt.text | ConvertFrom-Json).error.code -ne 'profile_in_use') { throw '重置错误未指出测试档正在使用。' }
    if (-not $testUi.CloseMainWindow() -or -not $testUi.WaitForExit(20000)) { throw '测试窗口没有正常关闭。' }
    $result = & $appFile.FullName @resetArgs | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or $result.status -ne 'reset') { throw '关闭测试窗口后无法重置。' }
} finally { Stop-TestProcess $testUi }

# 同一测试档可以通过文件、目录或环境变量打开。各入口即使收到更新器的环境变量也应直接关窗退出。
$profileDir = Get-Item -LiteralPath $result.directory
$identity = Get-Content -Raw -LiteralPath (Join-Path $profileDir.FullName 'profile.json') | ConvertFrom-Json
if ($identity.kind -ne 'test' -or $identity.id -ne $profileDir.Name) { throw '重置回执未指向登记的测试档。' }
$backupRoot = Get-Item -LiteralPath (Join-Path $profileRoot 'backups/profile-reset')
$tag = "$($identity.id)-1720000000000-t123456789abc"
$staging = New-Item -ItemType Directory -Path (Join-Path $backupRoot.FullName ".$tag.tmp")
Copy-Item -LiteralPath (Join-Path $profileDir.FullName 'profile.json') -Destination $staging.FullName
$marker = 'recovery-probe.txt'
[IO.File]::WriteAllText((Join-Path $profileDir.FullName $marker), '原测试档')
[IO.File]::WriteAllText((Join-Path $staging.FullName $marker), '待替换的种子')
$backup = Join-Path $backupRoot.FullName $tag
[IO.Directory]::Move($profileDir.FullName, $backup)
$recoverStart = New-AppStart
foreach ($arg in @('--data-root', $profileRoot, '--test-workspace')) { $recoverStart.ArgumentList.Add($arg) }
$recovered = [Diagnostics.Process]::Start($recoverStart)
try {
    $null = Wait-AppWindow $recovered ''
    if ([IO.File]::ReadAllText((Join-Path $profileDir.FullName $marker)) -ne '原测试档' -or
        [IO.File]::ReadAllText((Join-Path $staging.FullName $marker)) -ne '待替换的种子' -or
        (Test-Path -LiteralPath $backup)) { throw '打开测试窗口后未恢复中断前的完整目录。' }
    if (-not $recovered.CloseMainWindow() -or -not $recovered.WaitForExit(20000)) { throw '恢复后的测试窗口没有正常关闭。' }
    Write-Host '安装版测试入口已从中断的重置恢复原档，并正常关闭。'
} finally { Stop-TestProcess $recovered }
foreach ($entry in 'config', 'data-root', 'env') {
    $receiptDir = New-Item -ItemType Directory -Path (Join-Path $probe.FullName $entry)
    $testHealth = Join-Path $receiptDir.FullName 'healthy'
    $testStart = New-AppStart
    $testStart.Environment['OMNITRIGGER_UPDATE_HEALTH'] = $testHealth
    switch ($entry) {
        'config' {
            $testStart.ArgumentList.Add('--config')
            $testStart.ArgumentList.Add((Join-Path $profileDir.FullName 'bindings.json'))
        }
        'data-root' {
            $testStart.ArgumentList.Add('--data-root')
            $testStart.ArgumentList.Add($profileDir.FullName)
        }
        'env' { $testStart.Environment['OMNITRIGGER_HOME'] = $profileDir.FullName }
    }
    $testStart.ArgumentList.Add('--settings')
    $testUi = [Diagnostics.Process]::Start($testStart)
    try {
        $testWindow = Wait-AppWindow $testUi '' "OmniTrigger — 测试：$($identity.name)"
        # 留出输入后端就绪和界面轮询的时间，避免在首轮健康检查之前关窗。
        $timer = [Diagnostics.Stopwatch]::StartNew()
        while ($timer.Elapsed.TotalSeconds -lt 2 -and -not (Test-Path -LiteralPath $testHealth)) {
            Start-Sleep -Milliseconds 100
        }
        if (Test-Path -LiteralPath $testHealth) { throw "通过 $entry 打开的测试档误确认了安装版更新。" }
        if (-not [OmniTriggerUpgradeWindow]::PostMessage($testWindow, 0x0010, [UIntPtr]::Zero, [IntPtr]::Zero)) {
            throw "通过 $entry 打开的测试窗口未收到关闭消息。"
        }
        if (-not $testUi.WaitForExit(20000)) {
            $visible = [OmniTriggerUpgradeWindow]::IsWindowVisible($testWindow)
            throw "通过 $entry 打开的测试进程仍在运行。窗口可见：$visible，配置目录：$($result.directory)"
        }
        if ($testUi.ExitCode -ne 0) {
            throw "通过 $entry 打开的测试窗口退出码为 $($testUi.ExitCode)。"
        }
        if (Test-Path -LiteralPath $testHealth) { throw "通过 $entry 打开的测试档误确认了安装版更新。" }
    } finally { Stop-TestProcess $testUi }
}

$start = New-AppStart
$start.Environment['OMNITRIGGER_UPDATE_HEALTH'] = $health
$ui = [Diagnostics.Process]::Start($start)
try {
    $null = Close-ToTray $ui $health
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
    Stop-TestProcess $ui
}

foreach ($restart in '更新成功', '更新回滚', '主窗口打开设置', '设置返回主窗口') {
    $start = New-AppStart
    $reopen = New-AppStart
    switch ($restart) {
        '主窗口打开设置' { $reopen.ArgumentList.Add('--settings') }
        '设置返回主窗口' { $start.ArgumentList.Add('--settings') }
        default {
            foreach ($argument in @('--config', (Join-Path $data.FullName 'bindings.json'), '--settings')) {
                $start.ArgumentList.Add($argument)
            }
        }
    }
    $healthFile = $null
    if ($restart -eq '更新成功') {
        $receiptDir = New-Item -ItemType Directory -Path (Join-Path $probe.FullName 'update')
        $healthFile = Join-Path $receiptDir.FullName 'healthy'
        $start.Environment['OMNITRIGGER_UPDATE_HEALTH'] = $healthFile
    } elseif ($restart -eq '更新回滚') {
        $start.Environment['OMNITRIGGER_UPDATE_ERROR'] = '测试更新回滚'
    }
    $ui = [Diagnostics.Process]::Start($start)
    try {
        $window = Close-ToTray $ui $healthFile
        $launcher = [Diagnostics.Process]::Start($reopen)
        try {
            if (-not $launcher.WaitForExit(20000) -or $launcher.ExitCode -ne 0) {
                throw "${restart}时，重复启动的进程未正常退出。"
            }
            $timer = [Diagnostics.Stopwatch]::StartNew()
            while (-not $ui.HasExited -and -not [OmniTriggerUpgradeWindow]::IsWindowVisible($window) -and $timer.Elapsed.TotalSeconds -lt 10) {
                Start-Sleep -Milliseconds 100
            }
            [uint32]$owner = 0
            $null = [OmniTriggerUpgradeWindow]::GetWindowThreadProcessId($window, [ref]$owner)
            if ($ui.HasExited -or $owner -ne $ui.Id -or -not [OmniTriggerUpgradeWindow]::IsWindowVisible($window)) {
                throw "${restart}时，未能唤回原来的窗口。"
            }
            Write-Host "${restart}时，已唤回原托盘进程和窗口。"
        } finally { Stop-TestProcess $launcher }
    } finally { Stop-TestProcess $ui }
}
