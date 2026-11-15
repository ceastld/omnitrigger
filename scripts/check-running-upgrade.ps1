# 用于全新 CI 用户或独立 Windows 测试环境。安装器会使用这个用户的安装登记及开始菜单。
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Setup,
    [Parameter(Mandatory)][string]$Archive,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$DataRoot,
    [Parameter(Mandatory)][string]$ExpectedVersion
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
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
public static class OmniTriggerUpgradeWindow {
    private delegate bool EnumWindow(IntPtr hwnd, IntPtr parameter);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindow callback, IntPtr parameter);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr hwnd, StringBuilder title, int capacity);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, UIntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [StructLayout(LayoutKind.Sequential)] private struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    public static long WindowSize(IntPtr hwnd) {
        Rect r;
        if (!GetWindowRect(hwnd, out r)) throw new Exception("无法读取主窗口尺寸。");
        return ((long)(r.Right - r.Left) << 32) | (uint)(r.Bottom - r.Top);
    }
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern uint RegisterWindowMessage(string message);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint message, UIntPtr wParam, IntPtr lParam, uint flags, uint timeout, out UIntPtr result);
    public static IntPtr FindAppWindow(uint processId, string expectedTitle) {
        return FindAppWindow(processId, expectedTitle, true);
    }
    public static IntPtr FindAppWindow(uint processId, string expectedTitle, bool visibleOnly) {
        IntPtr workspace = IntPtr.Zero;
        EnumWindows((hwnd, unused) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != processId || (visibleOnly && !IsWindowVisible(hwnd))) return true;
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

    private delegate void WinEvent(IntPtr hook, uint kind, IntPtr hwnd, int objectId, int childId, uint thread, uint time);
    [DllImport("user32.dll")] private static extern IntPtr SetWinEventHook(uint first, uint last, IntPtr mod, WinEvent callback, uint pid, uint tid, uint flags);
    [DllImport("user32.dll")] private static extern bool UnhookWinEvent(IntPtr hook);
    [StructLayout(LayoutKind.Sequential)] private struct Message {
        public IntPtr Window; public uint Id; public UIntPtr WParam; public IntPtr LParam;
        public uint Time; public int X, Y; public uint Private;
    }
    [DllImport("user32.dll", EntryPoint="PeekMessageW")] private static extern bool PeekMessage(out Message m, IntPtr hwnd, uint first, uint last, uint remove);
    [DllImport("user32.dll", EntryPoint="DispatchMessageW")] private static extern IntPtr DispatchMessage(ref Message m);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [StructLayout(LayoutKind.Sequential)] private struct Point { public int X, Y; }
    [DllImport("user32.dll")] private static extern bool GetCursorPos(out Point p);
    [DllImport("user32.dll")] private static extern IntPtr MonitorFromPoint(Point p, uint flags);
    [DllImport("user32.dll")] private static extern IntPtr MonitorFromWindow(IntPtr h, uint flags);

    public static Process StartAndVerifyWindows(ProcessStartInfo start, bool requested, bool inTray) {
        const uint show = 0x8002; // EVENT_OBJECT_SHOW
        uint pid = 0;
        var shown = new HashSet<IntPtr>();
        WinEvent observe = (hook, kind, hwnd, obj, child, thread, time) => {
            if (pid == 0 || obj != 0 || child != 0 || hwnd == IntPtr.Zero) return;
            uint owner; GetWindowThreadProcessId(hwnd, out owner);
            if (owner == pid) shown.Add(hwnd);
        };
        // 先订阅再启动；即使同一毫秒内显示又隐藏，也能观察到首次显示。
        var hookHandle = SetWinEventHook(show, show, IntPtr.Zero, observe, 0, 0, 0);
        if (hookHandle == IntPtr.Zero) throw new Exception("无法观察窗口显示事件。");
        Process ui = null;
        try {
            ui = Process.Start(start);
            pid = (uint)ui.Id;
            var timer = Stopwatch.StartNew();
            var settled = new Stopwatch();
            while (!ui.HasExited && timer.Elapsed.TotalSeconds < 30) {
                Message m;
                while (PeekMessage(out m, IntPtr.Zero, 0, 0, 1)) DispatchMessage(ref m);
                var main = FindAppWindow(pid, "OmniTrigger", false);
                var panel = FindAppWindow(pid, "OmniTrigger 触发面板", false);
                bool visible = panel != IntPtr.Zero && IsWindowVisible(panel);
                if (!requested && (shown.Contains(panel) || visible))
                    throw new Exception("普通启动在呼出前显示了触发面板。");
                if (inTray && (shown.Contains(main) || IsWindowVisible(main)))
                    throw new Exception("托盘更新在用户主动唤回前显示了主窗口。");
                bool ready = main != IntPtr.Zero && (inTray || shown.Contains(main)) && panel != IntPtr.Zero && (!requested || (shown.Contains(panel) && visible));
                if (ready) settled.Start(); else settled.Reset();
                if (settled.ElapsedMilliseconds >= 750) {
                    if (requested) {
                        if (GetForegroundWindow() == panel) throw new Exception("首次呼出面板抢占了前台。");
                        Point cursor;
                        if (!GetCursorPos(out cursor) || MonitorFromWindow(panel, 0) != MonitorFromPoint(cursor, 2))
                            throw new Exception("首次呼出的触发面板未出现在指针所在屏幕。");
                    }
                    return ui;
                }
                Thread.Sleep(10);
            }
            throw new Exception("未观察到预期的主窗口和面板状态。");
        } catch {
            if (ui != null) {
                try { if (!ui.HasExited) { ui.Kill(); ui.WaitForExit(5000); } } finally { ui.Dispose(); }
            }
            throw;
        } finally {
            UnhookWinEvent(hookHandle);
            GC.KeepAlive(observe);
        }
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
    $null = $start.Environment.Remove('OMNITRIGGER_UPDATE_TRAY')
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

function Assert-TrayIdle([Diagnostics.Process]$ui, [IntPtr]$window) {
    # 以单个逻辑核计量。连续两个采样区间均超过半核才报告，给关闭动画和调度抖动留余量。
    $busyIntervals = 0
    foreach ($interval in 1..2) {
        $ui.Refresh()
        $before = $ui.TotalProcessorTime
        $elapsed = [Diagnostics.Stopwatch]::StartNew()
        foreach ($second in 1..4) {
            Start-Sleep -Seconds 1
            if ($ui.HasExited -or -not [OmniTriggerUpgradeWindow]::IsWindow($window) -or
                [OmniTriggerUpgradeWindow]::IsWindowVisible($window)) { throw '托盘采样期间主窗口重新出现或进程已退出。' }
        }
        $ui.Refresh()
        $cpu = ($ui.TotalProcessorTime - $before).TotalSeconds
        $wall = $elapsed.Elapsed.TotalSeconds
        Write-Host ('托盘空闲采样 {0}：{1:F2} 秒内使用了 {2:F3} 秒 CPU。' -f $interval, $wall, $cpu)
        if ($cpu -gt $wall / 2) { $busyIntervals++ }
    }
    if ($busyIntervals -eq 2) { throw '收起日常主窗口后，进程持续占用超过半个逻辑核。' }
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
$n10 = Join-Path $PSScriptRoot '../examples/actions/n10-rust'
$installedN10 = Join-Path $daily 'actions/clipboard-local'
$externalN10 = Join-Path $external 'actions/old-clipboard'
$null = New-Item -ItemType Directory -Force -Path (Split-Path $installedN10), (Split-Path $externalN10)
Copy-Item -LiteralPath $n10 -Destination $installedN10 -Recurse
Copy-Item -LiteralPath $n10 -Destination $externalN10 -Recurse
[IO.File]::WriteAllText((Join-Path $externalN10 'import-marker.txt'), 'from external copy')
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
$catalog = & $appFile.FullName --config $importDaily --list-package-actions | ConvertFrom-Json
$clipboard = @($catalog.actions | Where-Object { $_.call.package_id -ceq 'examples.n10-rust' })
if ($LASTEXITCODE -or $clipboard.Count -ne 2 -or 'toggle' -cnotin $clipboard.call.action_id -or
    (Test-Path -LiteralPath (Join-Path $daily 'actions/old-clipboard')) -or
    [IO.File]::ReadAllText((Join-Path $installedN10 'import-marker.txt')) -cne 'from external copy') {
    throw '从不同目录名导入后，剪贴板扩展未沿用日常安装目录或动作无法解析。'
}
Write-Host '已验证安装版从异名目录导入剪贴板扩展，原目录和 toggle 动作继续可用。'

# 恢复较早的日常备份后，已成功导入过的副本不应再次阻塞热键启动。
[IO.File]::WriteAllText($importDaily, '{"version":1,"bindings":[]}')
[IO.File]::SetLastWriteTimeUtc($importDaily, [DateTime]::UtcNow.AddDays(-1))
$restoredHash = (Get-FileHash -LiteralPath $importDaily).Hash
$importBackups = Join-Path $daily 'backups'
$savedCount = @(Get-ChildItem -LiteralPath $importBackups -Directory -Filter 'before-import-*').Count
foreach ($changed in @($false, $true)) {
    if ($changed) {
        [IO.File]::WriteAllText($sourceFile, '{"version":1, "bindings": []}')
        [IO.File]::SetLastWriteTimeUtc($sourceFile, [DateTime]::UtcNow.AddHours(5))
    }
    $ui = [Diagnostics.Process]::Start($ignoreStart)
    try {
        if ($changed) {
            $prompt = Wait-AppWindow $ui '' 'OmniTrigger 发现外部数据副本'
            if (-not [OmniTriggerUpgradeWindow]::PostMessage($prompt, 0x0111, [UIntPtr]2, [IntPtr]::Zero)) {
                throw '来源修改后的提示未能取消。'
            }
        }
        $null = Wait-AppWindow $ui '' 'OmniTrigger'
        if ((Get-FileHash -LiteralPath $importDaily).Hash -ne $restoredHash -or
            @(Get-ChildItem -LiteralPath $importBackups -Directory -Filter 'before-import-*').Count -ne $savedCount) {
            throw '恢复备份后，日常数据被覆盖或再次执行了导入。'
        }
    } finally { Stop-TestProcess $ui }
}
Write-Host '已验证恢复日常备份后直接启动，来源有新修改时才再次提示。'

# 保留上一步尚未处理的较新副本，后台启动仍需直接进入托盘。
$ignoreStart.ArgumentList.Add('--tray')
$quiet = [OmniTriggerUpgradeWindow]::StartAndVerifyWindows($ignoreStart, $false, $true)
try {
    $original = [OmniTriggerUpgradeWindow]::FindAppWindow($quiet.Id, 'OmniTrigger', $false)
    $repeat = [Diagnostics.Process]::Start($ignoreStart)
    try {
        if (-not $repeat.WaitForExit(20000) -or $repeat.ExitCode -ne 0 -or $quiet.HasExited -or
            [OmniTriggerUpgradeWindow]::IsWindowVisible($original) -or
            (Get-FileHash -LiteralPath $importDaily).Hash -ne $restoredHash) {
            throw '重复托盘启动改变了日常配置或显示了主窗口。'
        }
    } finally { Stop-TestProcess $repeat }
} finally { Stop-TestProcess $quiet }
Write-Host '已验证存在待导入的外部副本时，首次托盘启动和重复启动都保持隐藏。'

# 使用单独的数据根，验证安装版窗口确实持有测试档，正常退出后才允许重置。
$outerRoot = Join-Path $probe.FullName 'profiles-test'
$parentEntry = & $appFile.FullName --data-root $outerRoot --profile-create parent-test --seed empty | ConvertFrom-Json
if ($LASTEXITCODE -or $parentEntry.status -ne 'created') { throw '未能创建外层测试档。' }
$profileRoot = $parentEntry.directory
$testStart = New-AppStart
foreach ($arg in @('--data-root', $profileRoot, '--test-workspace')) { $testStart.ArgumentList.Add($arg) }
$testUi = [Diagnostics.Process]::Start($testStart)
try {
    $null = Wait-AppWindow $testUi ''
    $resetArgs = @('--data-root', $profileRoot, '--profile-reset', 'testing')
    foreach ($case in @(@{root=$profileRoot; alias='testing'}, @{root=$outerRoot; alias='parent-test'})) {
        $resetError = Join-Path $probe.FullName "reset-$($case.alias).json"
        $reset = New-AppStart
        foreach ($arg in @('--data-root', $case.root, '--profile-reset', $case.alias, '--relaunch-result', $resetError)) {
            $reset.ArgumentList.Add($arg)
        }
        $attempt = [Diagnostics.Process]::Start($reset)
        try {
            if (-not $attempt.WaitForExit(20000) -or $attempt.ExitCode -ne 1) { throw "窗口打开时未拒绝重置 $($case.alias)。" }
        } finally { Stop-TestProcess $attempt }
        $receipt = Get-Content -Raw -LiteralPath $resetError | ConvertFrom-Json
        if ($receipt.ok -or ($receipt.text | ConvertFrom-Json).error.code -ne 'profile_in_use') {
            throw "重置错误未指出 $($case.alias) 正在使用。"
        }
    }
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
    $testStart.Environment['OMNITRIGGER_UPDATE_TRAY'] = '1'
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

foreach ($requested in @($false, $true)) {
    $panelStart = New-AppStart
    $panelStart.Environment['OMNITRIGGER_HOME'] = Join-Path $probe.FullName '面板启动'
    if ($requested) { $panelStart.ArgumentList.Add('--panel') }
    Stop-TestProcess ([OmniTriggerUpgradeWindow]::StartAndVerifyWindows($panelStart, $requested, $false))
}
Write-Host '已验证预热面板保持隐藏，首次呼出显示在指针所在屏幕并保留前台。'

$testTray = New-AppStart
foreach ($arg in @('--config', (Join-Path $profileDir.FullName 'bindings.json'), '--tray')) {
    $testTray.ArgumentList.Add($arg)
}
$testTray.RedirectStandardError = $true
$testTray.StandardErrorEncoding = [Text.Encoding]::UTF8
$rejected = [Diagnostics.Process]::Start($testTray)
try {
    $reason = $rejected.StandardError.ReadToEndAsync()
    if (-not $rejected.WaitForExit(20000) -or $rejected.ExitCode -ne 1 -or
        $reason.Result -notmatch '只用于日常工作区') { throw '已登记的测试档未拒绝后台托盘启动。' }
} finally { Stop-TestProcess $rejected }

$start = New-AppStart
$start.Environment['OMNITRIGGER_UPDATE_HEALTH'] = $health
$ui = [Diagnostics.Process]::Start($start)
try {
    $dailyWindow = Close-ToTray $ui $health
    Assert-TrayIdle $ui $dailyWindow
    # 快捷方式参数拼错或脚本查询版本时，命令应独立结束，已收起的日常窗口仍由原进程持有。
    foreach ($flag in '--version', '--help', '--check-confg') {
        $query = New-AppStart
        $query.ArgumentList.Add($flag)
        $query.CreateNoWindow = $true
        $query.RedirectStandardOutput = $true
        $query.RedirectStandardError = $true
        $query.StandardOutputEncoding = $query.StandardErrorEncoding = [Text.Encoding]::UTF8
        $child = [Diagnostics.Process]::Start($query)
        try {
            $stdout = $child.StandardOutput.ReadToEndAsync()
            $stderr = $child.StandardError.ReadToEndAsync()
            if (-not $child.WaitForExit(10000)) { throw "$flag 未在后台退出。" }
            if ($flag -eq '--check-confg') {
                $receipt = $stderr.Result | ConvertFrom-Json
                if ($child.ExitCode -ne 1 -or $stdout.Result -ne '' -or $receipt.error.code -ne 'invalid_cli_arguments' -or
                    $receipt.error.message -notlike "*$flag*") { throw "$flag 未返回具体参数错误。" }
            } else {
                if ($child.ExitCode -ne 0 -or $stderr.Result -ne '' -or
                    ($flag -eq '--version' -and $stdout.Result.Trim() -cne "OmniTrigger $ExpectedVersion") -or
                    ($flag -eq '--help' -and $stdout.Result -notmatch '--test-workspace')) { throw "$flag 的输出或退出码错误。" }
            }
            if ($ui.HasExited -or -not [OmniTriggerUpgradeWindow]::IsWindow($dailyWindow) -or
                [OmniTriggerUpgradeWindow]::IsWindowVisible($dailyWindow)) { throw "$flag 改变了日常窗口的托盘状态。" }
        } finally { Stop-TestProcess $child }
    }
    # 日常窗口已经确认 Hook 就绪。另一进程的真实监听请求应在安装 Hook 前返回占用原因。
    $listen = New-AppStart
    foreach ($arg in @('--data-root', $profileRoot, '--test-workspace', '--listen', '--smoke-test')) {
        $listen.ArgumentList.Add($arg)
    }
    $listen.RedirectStandardError = $true
    $listen.StandardErrorEncoding = [Text.Encoding]::UTF8
    $listen.CreateNoWindow = $true
    $attempt = [Diagnostics.Process]::Start($listen)
    try {
        $message = $attempt.StandardError.ReadToEndAsync()
        if (-not $attempt.WaitForExit(20000)) { throw '日常窗口收起时，测试档的监听检查超时。' }
        if ($attempt.ExitCode -ne 1 -or $message.Result -notmatch '另一个 OmniTrigger 实例正在监听') {
            throw "测试档的监听请求未报告占用。退出码 $($attempt.ExitCode)，回执 $($message.Result)"
        }
    } finally { Stop-TestProcess $attempt }
    $edit = New-AppStart
    foreach ($arg in @('--data-root', $profileRoot, '--test-workspace', '--listen')) {
        $edit.ArgumentList.Add($arg)
    }
    $editor = [Diagnostics.Process]::Start($edit)
    try {
        $null = Wait-AppWindow $editor '' "OmniTrigger — 测试：$($identity.name)"
        if (-not $editor.CloseMainWindow() -or -not $editor.WaitForExit(20000) -or $editor.ExitCode -ne 0) {
            throw '监听被占用时，测试配置窗口没有正常关闭。'
        }
    } finally { Stop-TestProcess $editor }
    if ($ui.HasExited -or [OmniTriggerUpgradeWindow]::IsWindowVisible($dailyWindow)) {
        throw '测试档的监听请求影响了日常托盘窗口。'
    }
    $install = [Diagnostics.ProcessStartInfo]::new($setupFile.FullName)
    $install.UseShellExecute = $false
    $receiptDir = New-Item -ItemType Directory -Path (Join-Path $probe.FullName 'after-install')
    $receipt = Join-Path $receiptDir.FullName 'healthy'
    $install.Environment['OMNITRIGGER_HOME'] = $data.FullName
    $install.Environment['OMNITRIGGER_UPDATE_HEALTH'] = $receipt
    $started = [DateTime]::Now
    foreach ($argument in @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CLOSEAPPLICATIONS',
        '/NORESTARTAPPLICATIONS', '/STARTINTRAY=1', "/DIR=$($appFile.DirectoryName)", "/LOG=$log")) {
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
    $replacement = $null
    try {
        $timer = [Diagnostics.Stopwatch]::StartNew()
        do {
            if (-not $replacement) {
                $matchesInstall = @(Get-Process -Name $appFile.BaseName -ErrorAction SilentlyContinue |
                    Where-Object { $_.Path -eq $appFile.FullName -and $_.Id -ne $ui.Id -and $_.StartTime -ge $started })
                if ($matchesInstall.Count -gt 1) { throw '安装器启动了多个日常进程。' }
                $replacement = $matchesInstall | Select-Object -First 1
            }
            if ($replacement -and (Test-Path -LiteralPath $receipt -PathType Leaf)) { break }
            Start-Sleep -Milliseconds 150
        } while ($timer.Elapsed.TotalSeconds -lt 45)
        if (-not $replacement -or $replacement.HasExited -or
            -not (Test-Path -LiteralPath $receipt -PathType Leaf) -or
            [IO.File]::ReadAllText($receipt) -cne $ExpectedVersion) { throw '覆盖安装后新版日常监听未就绪。' }
        $hidden = [OmniTriggerUpgradeWindow]::FindAppWindow($replacement.Id, 'OmniTrigger', $false)
        if ($hidden -eq [IntPtr]::Zero -or [OmniTriggerUpgradeWindow]::IsWindowVisible($hidden)) {
            throw '覆盖安装后新版主窗口未收在托盘。'
        }
        $launcher = [Diagnostics.Process]::Start((New-AppStart))
        try {
            if (-not $launcher.WaitForExit(20000) -or $launcher.ExitCode -ne 0 -or
                (Wait-AppWindow $replacement $receipt 'OmniTrigger') -ne $hidden) {
                throw '普通启动未能唤回原日常配置的托盘窗口。'
            }
        } finally { Stop-TestProcess $launcher }
        Write-Host '已验证静默覆盖安装后，新版在托盘接手日常监听，普通启动唤回同一窗口。'
    } finally { if ($replacement) { Stop-TestProcess $replacement } }
} finally {
    # 只收尾本脚本创建并持有句柄的测试进程；上述断言在收尾之前执行。
    Stop-TestProcess $ui
}

$takeover = New-AppStart
$takeoverRoot = New-Item -ItemType Directory -Path (Join-Path $probe.FullName 'listener-takeover')
$takeover.Environment['OMNITRIGGER_HOME'] = $takeoverRoot.FullName
$takeoverHealth = Join-Path $takeoverRoot.FullName 'healthy'
$takeover.Environment['OMNITRIGGER_UPDATE_HEALTH'] = $takeoverHealth
$next = [Diagnostics.Process]::Start($takeover)
try {
    $null = Wait-AppWindow $next $takeoverHealth 'OmniTrigger'
    Write-Host '已验证日常托盘监听不会被测试档抢占，旧进程退出后其他配置可正常监听。'
} finally { Stop-TestProcess $next }

foreach ($restart in '更新成功', '更新回滚', '托盘更新成功', '托盘更新回滚', '手动打开设置', '主窗口打开设置', '设置返回主窗口') {
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
    if ($restart.EndsWith('更新成功')) {
        $receiptDir = New-Item -ItemType Directory -Path (Join-Path $probe.FullName $restart)
        $healthFile = Join-Path $receiptDir.FullName 'healthy'
        $start.Environment['OMNITRIGGER_UPDATE_HEALTH'] = $healthFile
    } elseif ($restart.EndsWith('更新回滚')) {
        $start.Environment['OMNITRIGGER_UPDATE_ERROR'] = '测试更新回滚'
    }
    $inTray = $restart.StartsWith('托盘')
    if ($inTray -or $restart -eq '手动打开设置') { $start.Environment['OMNITRIGGER_UPDATE_TRAY'] = '1' }
    $ui = if ($inTray) {
        [OmniTriggerUpgradeWindow]::StartAndVerifyWindows($start, $false, $true)
    } else { [Diagnostics.Process]::Start($start) }
    try {
        if ($inTray) {
            $hidden = [OmniTriggerUpgradeWindow]::FindAppWindow($ui.Id, 'OmniTrigger', $false)
            if ($healthFile) {
                $timer = [Diagnostics.Stopwatch]::StartNew()
                while (-not $ui.HasExited -and -not (Test-Path -LiteralPath $healthFile) -and $timer.Elapsed.TotalSeconds -lt 30) {
                    Start-Sleep -Milliseconds 100
                }
                if ([IO.File]::ReadAllText($healthFile) -cne $ExpectedVersion) { throw '后台重启未在托盘完成输入后端健康确认。' }
            }
            $duplicate = [Diagnostics.Process]::Start($start)
            try {
                if (-not $duplicate.WaitForExit(10000) -or $duplicate.ExitCode -ne 0) { throw '重复后台重启未退出。' }
            } finally { Stop-TestProcess $duplicate }
            if ($ui.HasExited -or [OmniTriggerUpgradeWindow]::IsWindowVisible($hidden)) { throw '重复后台重启唤起了窗口。' }
            # 模拟通知区域对该图标发出的 NIN_SELECT 回调。
            if (-not [OmniTriggerUpgradeWindow]::PostMessage($hidden, 0x834a, [UIntPtr]::Zero, [IntPtr]0x10400)) {
                throw '托盘选择消息未送达。'
            }
            $restored = Wait-AppWindow $ui '' 'OmniTrigger'
            $timer = [Diagnostics.Stopwatch]::StartNew()
            while ([OmniTriggerUpgradeWindow]::WindowSize($restored) -ne $normalSize -and $timer.Elapsed.TotalSeconds -lt 10) {
                Start-Sleep -Milliseconds 100
            }
            if ($restored -ne $hidden -or [OmniTriggerUpgradeWindow]::WindowSize($restored) -ne $normalSize) {
                throw '从托盘首次打开时没有恢复正常大小的原窗口。'
            }
        }
        $window = Close-ToTray $ui $healthFile
        if ($restart -eq '更新成功') { $normalSize = [OmniTriggerUpgradeWindow]::WindowSize($window) }
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

# 重复启动应只唤回原进程，导入恢复留给下一次真正打开工作区。
$wakeRoot = New-Item -ItemType Directory -Path (Join-Path $probe.FullName 'wake-during-import')
$bindings = Join-Path $wakeRoot.FullName 'bindings.json'
$beforeImport = '{"version":24,"bindings":[]}'
$imported = Get-Item -LiteralPath (Join-Path $data.FullName 'bindings.json')
Copy-Item -LiteralPath $imported.FullName -Destination $bindings
$incomingHash = [Security.Cryptography.SHA256]::HashData([IO.File]::ReadAllBytes($bindings))
$wake = New-AppStart
$wake.Environment['OMNITRIGGER_HOME'] = $wakeRoot.FullName
$ui = [Diagnostics.Process]::Start($wake)
try {
    $window = Close-ToTray $ui ''
    $pending = Join-Path $wakeRoot.FullName 'backups/external-import.pending'
    $null = New-Item -ItemType Directory -Path (Join-Path $pending 'incoming'), (Join-Path $pending 'originals') -Force
    [IO.File]::WriteAllText((Join-Path $pending 'originals/bindings.json'), $beforeImport)
    $plan = @{ backup = 'before-import-wake'; directories = @(); files = @(@{path = 'bindings.json'; sha256 = [int[]]$incomingHash}) }
    [IO.File]::WriteAllText((Join-Path $pending 'plan.json'), ($plan | ConvertTo-Json -Depth 5))
    $launcher = [Diagnostics.Process]::Start($wake)
    try {
        if (-not $launcher.WaitForExit(20000) -or $launcher.ExitCode -ne 0) { throw '有待恢复的导入时，重复启动未正常返回。' }
        if ((Wait-AppWindow $ui '' 'OmniTrigger') -ne $window) { throw '未唤回原托盘窗口。' }
        if (-not (Test-Path -LiteralPath (Join-Path $pending 'plan.json')) -or
            (Get-FileHash -LiteralPath $bindings).Hash -ne [Convert]::ToHexString($incomingHash)) {
            throw '唤回托盘时提前恢复了正在使用的热键文件。'
        }
    } finally { Stop-TestProcess $launcher }
} finally { Stop-TestProcess $ui }
$ui = [Diagnostics.Process]::Start($wake)
try {
    $null = Wait-AppWindow $ui '' 'OmniTrigger'
    if ((Test-Path -LiteralPath $pending) -or [IO.File]::ReadAllText($bindings) -cne $beforeImport) {
        throw '重新打开日常窗口时未恢复导入前的热键。'
    }
    Write-Host '已验证唤回托盘保留当前热键，日常实例退出后再启动会恢复导入前的数据。'
} finally { Stop-TestProcess $ui }
