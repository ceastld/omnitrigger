# 无网络的真实 Windows 更新集成测试；临时目录和进程均由本测试创建。
param([Parameter(Mandatory)][string]$FixtureRoot)
. (Join-Path $PSScriptRoot 'update_common.ps1')
$updateTestRoot = [IO.Path]::GetFullPath($FixtureRoot)
Assert-UpdatePath $updateTestRoot
$null = [IO.Directory]::CreateDirectory($updateTestRoot)
$updateTestChildren = New-Object 'Collections.Generic.List[Diagnostics.Process]'
function Assert-Test([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Expect-Failure([scriptblock]$Action) {
    $updateRejected = $false
    try { & $Action } catch { $updateRejected = $true }
    Assert-Test $updateRejected '应当拒绝不安全的更新。'
}
function New-TestZip([string]$Name, [string[]]$Entries) {
    $updateZipPath = Join-Path $updateTestRoot "$Name.zip"
    $updateZip = [IO.Compression.ZipFile]::Open($updateZipPath, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($updateName in $Entries) {
            $updateEntry = $updateZip.CreateEntry($updateName)
            if ($Name -eq 'symlink') { $updateEntry.ExternalAttributes = -1577058304 } # Unix symlink 0120000
            $updateWriter = New-Object IO.StreamWriter($updateEntry.Open())
            try { $updateWriter.Write('fixture') } finally { $updateWriter.Dispose() }
        }
    } finally { $updateZip.Dispose() }
    return $updateZipPath
}
function Start-TestProcess([string]$File, [string[]]$Arguments, [string]$Directory) {
    $updateInfo = New-Object Diagnostics.ProcessStartInfo
    $updateInfo.FileName = $File
    $updateInfo.Arguments = $Arguments -join ' '
    $updateInfo.WorkingDirectory = $Directory
    $updateInfo.UseShellExecute = $false
    $updateInfo.CreateNoWindow = $true
    $updateProcess = New-Object Diagnostics.Process
    $updateProcess.StartInfo = $updateInfo
    $null = $updateProcess.Start()
    $updateTestChildren.Add($updateProcess)
    return $updateProcess
}
try {
    foreach ($updateCase in @('traversal', 'duplicate', 'symlink', 'missing')) {
        $updateEntries = switch ($updateCase) {
            'traversal' { @('omnitrigger.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'licenses/../../outside') }
            'duplicate' { @('omnitrigger.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'licenses/a', 'licenses/A') }
            'symlink' { @('omnitrigger.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.md') }
            'missing' { @('LICENSE') }
        }
        $updateZipPath = New-TestZip $updateCase $updateEntries
        $updateDestination = Join-Path $updateTestRoot $updateCase
        Expect-Failure { Expand-UpdateArchive $updateZipPath $updateDestination }
        Assert-Test (-not (Test-Path -LiteralPath $updateDestination)) '包预检失败时不能写入解压目录。'
    }
    $updateZipPath = New-TestZip 'valid' @('omnitrigger.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'licenses/依赖 许可/LICENSE.txt')
    $updateFiles = Expand-UpdateArchive $updateZipPath (Join-Path $updateTestRoot 'valid')
    Assert-Test ($updateFiles.Count -eq 4) '合法发行包解压失败。'
    $updateHashPath = Join-Path $updateTestRoot 'SHA256SUMS.txt'
    [IO.File]::WriteAllText($updateHashPath, ('0' * 64 + '  valid.zip'))
    Expect-Failure { Confirm-UpdateHash $updateHashPath 'valid.zip' $updateZipPath }
    [IO.File]::WriteAllText($updateHashPath, ((Get-UpdateHash $updateZipPath) + '  valid.zip'))
    Confirm-UpdateHash $updateHashPath 'valid.zip' $updateZipPath
    Confirm-UpdateDigest $updateZipPath (Get-UpdateHash $updateZipPath)
    Expect-Failure { Confirm-UpdateDigest $updateZipPath ('f' * 64) }

    # 用极小的真实 EXE 模拟旧版、健康新版及启动失败，经过生产替换器而非测试替身。
    $updateFixtureExe = Join-Path $updateTestRoot 'fixture.exe'
    $updateSource = @'
using System;
using System.IO;
using System.Diagnostics;
using System.Threading;
class Fixture {
    static int Main(string[] args) {
        string root = AppDomain.CurrentDomain.BaseDirectory;
        if (args.Length == 1 && args[0] == "--parent") {
            while (!File.Exists(Path.Combine(root, "exit-parent"))) Thread.Sleep(50);
            return 0;
        }
        string version = File.ReadAllText(Path.Combine(root, "README.txt"));
        if (version == "bad") return 1;
        string health = Environment.GetEnvironmentVariable("OMNITRIGGER_UPDATE_HEALTH");
        if (!String.IsNullOrEmpty(health)) File.WriteAllText(health, "0.2.0");
        // 测试端以回执文件出现作为就绪信号，发布前先写完并关闭句柄。
        string receipt = args[1] + ".tmp";
        File.WriteAllText(receipt, version + "|" + Process.GetCurrentProcess().Id + "|" + Environment.GetEnvironmentVariable("OMNITRIGGER_UPDATE_ERROR"));
        File.Move(receipt, args[1]);
        Thread.Sleep(60000);
        return 0;
    }
}
'@
    Add-Type -TypeDefinition $updateSource -OutputAssembly $updateFixtureExe -OutputType WindowsApplication
    foreach ($updateCase in @('success', 'rollback', 'tampered', 'cancelled')) {
        $updateRoot = Join-Path $updateTestRoot "安装 空格-$updateCase"
        $updateStage = Join-Path $updateRoot '.omnitrigger-update-fixture'
        $updatePayload = Join-Path $updateStage 'payload'
        $null = [IO.Directory]::CreateDirectory($updatePayload)
        $updateExe = Join-Path $updateRoot 'omnitrigger.exe'
        Copy-Item -LiteralPath $updateFixtureExe -Destination $updateExe
        Copy-Item -LiteralPath $updateFixtureExe -Destination (Join-Path $updatePayload 'omnitrigger.exe')
        [IO.File]::WriteAllText((Join-Path $updateRoot 'README.txt'), 'old')
        [IO.File]::WriteAllText((Join-Path $updatePayload 'README.txt'), $(if ($updateCase -eq 'rollback') { 'bad' } else { 'new' }))
        $updateConfig = Join-Path $updateRoot '隔离 配置.json'
        $updateParent = Start-TestProcess $updateExe @('--parent') $updateRoot
        $updateManifest = @('omnitrigger.exe', 'README.txt') | ForEach-Object {
            @{ name = $_; sha256 = (Get-UpdateHash (Join-Path $updatePayload $_)) }
        }
        if ($updateCase -eq 'tampered') { [IO.File]::WriteAllText((Join-Path $updatePayload 'README.txt'), 'changed') }
        $updatePlan = @{ executable = $updateExe; config = $updateConfig; parent_pid = $updateParent.Id; release = @{ tag_name = 'v0.2.0' }; token = 'fixture-session' }
        [IO.File]::WriteAllText((Join-Path $updateStage 'plan.json'), ($updatePlan | ConvertTo-Json -Depth 5))
        [IO.File]::WriteAllText((Join-Path $updateStage 'files.json'), (ConvertTo-Json -InputObject @($updateManifest)))
        foreach ($updateScript in @('update_apply.ps1', 'update_common.ps1')) {
            $updateOutputName = if ($updateScript -eq 'update_apply.ps1') { 'apply.ps1' } else { 'common.ps1' }
            [IO.File]::WriteAllText((Join-Path $updateStage $updateOutputName), ([char]0xFEFF + [IO.File]::ReadAllText((Join-Path $PSScriptRoot $updateScript))))
        }
        $env:OMNITRIGGER_UPDATE_STAGE = $updateStage
        $updateHelper = Start-TestProcess 'powershell.exe' @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', ('"' + (Join-Path $updateStage 'apply.ps1') + '"')) $updateRoot
        $updateDeadline = [DateTime]::UtcNow.AddSeconds(15)
        while (-not (Test-Path -LiteralPath (Join-Path $updateStage 'helper-ready')) -and -not $updateHelper.HasExited -and [DateTime]::UtcNow -lt $updateDeadline) { Start-Sleep -Milliseconds 50 }
        if ($updateCase -eq 'tampered') {
            Assert-Test ($updateHelper.WaitForExit(5000) -and $updateHelper.ExitCode -ne 0) '暂存内容被修改时必须拒绝更新。'
            Assert-Test (-not $updateParent.HasExited) '预检失败不能退出旧版。'
            Assert-Test ([IO.File]::ReadAllText((Join-Path $updateRoot 'README.txt')) -ceq 'old') '预检失败不能替换旧文件。'
            $updateParent.Kill(); $updateParent.WaitForExit()
            continue
        }
        Assert-Test (Test-Path -LiteralPath (Join-Path $updateStage 'helper-ready')) '替换器未就绪。'
        Assert-Test ([IO.File]::ReadAllText((Join-Path $updateRoot 'README.txt')) -ceq 'old') '旧版退出前不能替换文件。'
        if ($updateCase -ne 'cancelled') { [IO.File]::WriteAllText((Join-Path $updateStage 'authorize'), $updatePlan.token) }
        [IO.File]::WriteAllText((Join-Path $updateRoot 'exit-parent'), 'exit')
        Assert-Test ($updateHelper.WaitForExit(45000)) '替换器未能及时结束。'
        if ($updateCase -eq 'cancelled') {
            Assert-Test ($updateHelper.ExitCode -ne 0) '未授权的普通退出不能安装更新。'
            Assert-Test ([IO.File]::ReadAllText((Join-Path $updateRoot 'README.txt')) -ceq 'old') '未授权时不能替换原文件。'
            continue
        }
        $updateDeadline = [DateTime]::UtcNow.AddSeconds(5)
        while (-not (Test-Path -LiteralPath $updateConfig) -and [DateTime]::UtcNow -lt $updateDeadline) { Start-Sleep -Milliseconds 50 }
        $updateResult = [IO.File]::ReadAllText($updateConfig).Split('|')
        $updateChildProcess = Get-Process -Id ([int]$updateResult[1])
        $updateTestChildren.Add($updateChildProcess)
        if ($updateCase -eq 'success') {
            Assert-Test ($updateHelper.ExitCode -eq 0 -and $updateResult[0] -ceq 'new') '更新后没有成功启动新版。'
            Assert-Test (-not (Test-Path -LiteralPath $updateStage)) '成功后应清理暂存和备份。'
        } else {
            Assert-Test ($updateHelper.ExitCode -ne 0 -and $updateResult[0] -ceq 'old') '新版启动失败后没有重启旧版。'
            Assert-Test ($updateResult[2] -match '更新失败') '回滚后的旧版应收到错误原因。'
            Assert-Test ([IO.File]::ReadAllText((Join-Path $updateRoot 'README.txt')) -ceq 'old') '回滚没有恢复原文件。'
        }
        $updateChildProcess.Kill(); $updateChildProcess.WaitForExit()
    }
    Write-Host '更新集成测试通过：SHA-256、ZIP 路径、预检、退出交接、中文路径、重启与失败回滚。'
} finally {
    foreach ($updateProcess in $updateTestChildren) {
        if (-not $updateProcess.HasExited) { $updateProcess.Kill(); $updateProcess.WaitForExit() }
        $updateProcess.Dispose()
    }
    Remove-Item Env:OMNITRIGGER_UPDATE_STAGE -ErrorAction SilentlyContinue
}
