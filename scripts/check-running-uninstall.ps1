param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Archive,
    [Parameter(Mandatory)][string]$ExpectedVersion
)
$ErrorActionPreference = 'Stop'
$exe = Get-Item -LiteralPath $Executable
$root = New-Item -ItemType Directory -Path (Join-Path $env:RUNNER_TEMP ('omnitrigger-uninstall-' + [guid]::NewGuid().ToString('N')))
$portable = Join-Path $root.FullName 'portable'
Expand-Archive -LiteralPath $Archive -DestinationPath $portable
$children = [Collections.Generic.List[Diagnostics.Process]]::new()
$uninstaller = Join-Path $exe.DirectoryName 'unins000.exe'
$flags = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')

function Start-TestWindow([string]$image, [string]$dataRoot) {
    $start = [Diagnostics.ProcessStartInfo]::new($image)
    $start.UseShellExecute = $false
    foreach ($arg in @('--data-root', $dataRoot, '--test-workspace')) { $start.ArgumentList.Add($arg) }
    $ui = [Diagnostics.Process]::Start($start)
    $children.Add($ui)
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        $ui.Refresh()
        if ($ui.HasExited) { throw "测试窗口提前退出：$image" }
        if ($ui.MainWindowHandle -ne [IntPtr]::Zero -and $ui.Path -ieq $image) { return $ui }
        Start-Sleep -Milliseconds 100
    } while ($timer.Elapsed.TotalSeconds -lt 60)
    throw "测试窗口没有就绪：$image"
}

try {
    $reader = [IO.File]::Open($exe.FullName, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $blocked = Start-Process $uninstaller -ArgumentList $flags -WindowStyle Hidden -Wait -PassThru
        if ($blocked.ExitCode -eq 0 -or -not (Test-Path -LiteralPath $uninstaller) -or -not (Test-Path -LiteralPath $exe.FullName)) {
            throw '文件被其他程序占用时，卸载器没有保留完整的重试入口。'
        }
    } finally { $reader.Dispose() }
    $start = [Diagnostics.ProcessStartInfo]::new($exe.FullName)
    $start.UseShellExecute = $false
    $start.ArgumentList.Add('--tray')
    $health = Join-Path $root.FullName 'healthy'
    $start.Environment['OMNITRIGGER_UPDATE_HEALTH'] = $health
    $daily = [Diagnostics.Process]::Start($start)
    $children.Add($daily)
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath $health) -and -not $daily.HasExited -and $timer.Elapsed.TotalSeconds -lt 60) {
        Start-Sleep -Milliseconds 100
    }
    if ($daily.HasExited -or [IO.File]::ReadAllText($health) -cne $ExpectedVersion -or $daily.MainWindowHandle -ne [IntPtr]::Zero) {
        throw '卸载前日常实例未进入托盘并开始监听。'
    }
    $installedTest = Start-TestWindow $exe.FullName (Join-Path $root.FullName 'installed-profile')
    $portableTest = Start-TestWindow (Join-Path $portable 'omnitrigger.exe') (Join-Path $root.FullName 'portable-profile')
    $uninstall = Start-Process $uninstaller -ArgumentList $flags -WindowStyle Hidden -Wait -PassThru
    if ($uninstall.ExitCode -or -not $daily.WaitForExit(5000) -or -not $installedTest.WaitForExit(5000) -or (Test-Path -LiteralPath $exe.FullName)) {
        throw '卸载后仍有安装版进程或程序文件。'
    }
    $portableTest.Refresh()
    if ($portableTest.HasExited -or -not $portableTest.Responding -or $portableTest.MainWindowHandle -eq [IntPtr]::Zero) {
        throw '卸载影响了其他目录的便携版。'
    }
    Write-Host '卸载已结束安装版的托盘监听和测试窗口，另一个目录的便携版仍在运行。'
} finally {
    foreach ($child in $children) {
        try { if (-not $child.HasExited) { $child.Kill(); $null = $child.WaitForExit(5000) } }
        finally { $child.Dispose() }
    }
}
