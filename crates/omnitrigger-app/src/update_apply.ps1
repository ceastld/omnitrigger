# 独立于主进程等待退出；失败回滚已替换文件，再重新启动旧版。
. (Join-Path $PSScriptRoot 'common.ps1')
$updateChanged = New-Object 'Collections.Generic.List[object]'
$updateChild = $null
$updateParentExited = $false
function Invoke-UpdateFileOperation([scriptblock]$Action) {
    # 进程退出后，映像或防病毒扫描的共享锁可能短暂保留；有界重试，不结束其他进程。
    $updateRetryDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while ($true) {
        try { & $Action; return } catch [IO.IOException] {
            if ([DateTime]::UtcNow -ge $updateRetryDeadline) { throw }
            Start-Sleep -Milliseconds 100
        } catch [UnauthorizedAccessException] {
            if ([DateTime]::UtcNow -ge $updateRetryDeadline) { throw }
            Start-Sleep -Milliseconds 100
        }
    }
}
function Start-UpdatedApplication([bool]$HealthCheck) {
    if ($HealthCheck) { $env:OMNITRIGGER_UPDATE_HEALTH = Join-Path $updateStage 'healthy' }
    else { Remove-Item Env:OMNITRIGGER_UPDATE_HEALTH -ErrorAction SilentlyContinue }
    try {
        # Windows 文件名不能包含双引号；配置路径作为完整独立参数引用。
        # 这是用户需要看到的交互界面；下载器和替换器自身保持隐藏。
        return Start-Process -FilePath $updatePlan.executable -ArgumentList @('--config', ('"' + $updatePlan.config + '"'), '--settings') -WorkingDirectory $updateRoot -WindowStyle Normal -PassThru
    } finally { Remove-Item Env:OMNITRIGGER_UPDATE_HEALTH -ErrorAction SilentlyContinue }
}
try {
    $updateStage = [IO.Path]::GetFullPath($env:OMNITRIGGER_UPDATE_STAGE)
    Assert-UpdatePath $updateStage
    $updatePlan = [IO.File]::ReadAllText((Join-Path $updateStage 'plan.json')) | ConvertFrom-Json
    $updateRoot = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($updatePlan.executable))
    if ([IO.Path]::GetDirectoryName($updateStage) -ine $updateRoot -or [IO.Path]::GetFileName($updatePlan.executable) -cne 'omnitrigger.exe') { throw '更新目标与暂存目录不符。' }
    Assert-UpdatePath $updateRoot
    $updateFiles = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText((Join-Path $updateStage 'files.json')))
    if (@($updateFiles | Where-Object name -CEQ 'omnitrigger.exe').Count -ne 1) { throw '更新清单缺少程序。' }
    foreach ($updateFile in $updateFiles) {
        if (-not (Test-UpdateRelativePath $updateFile.name)) { throw '更新清单路径无效。' }
        $updateSource = Join-Path (Join-Path $updateStage 'payload') $updateFile.name
        $updateTarget = Join-Path $updateRoot $updateFile.name
        Assert-UpdatePath $updateSource; Assert-UpdatePath $updateTarget
        if ((Get-UpdateHash $updateSource) -ine $updateFile.sha256) { throw '待安装文件已发生变化。' }
    }
    $updateParent = Get-Process -Id $updatePlan.parent_pid -ErrorAction SilentlyContinue
    if ($null -eq $updateParent -or $updateParent.Path -ine $updatePlan.executable) { throw '原应用已退出，已取消更新。' }
    if (-not $updatePlan.token) { throw '缺少本次更新的退出授权身份。' }
    [IO.File]::WriteAllText((Join-Path $updateStage 'helper-ready'), 'ready')
    $updateAuthorization = Join-Path $updateStage 'authorize'
    $updateAuthorizationDeadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path -LiteralPath $updateAuthorization)) {
        if ($updateParent.HasExited -or [DateTime]::UtcNow -gt $updateAuthorizationDeadline) { throw '未收到本次更新的退出授权，已取消更新。' }
        Start-Sleep -Milliseconds 100
    }
    Assert-UpdatePath $updateAuthorization
    if ([IO.File]::ReadAllText($updateAuthorization) -cne $updatePlan.token) { throw '本次更新的退出授权身份不匹配。' }
    if (-not $updateParent.WaitForExit(30000)) { throw '应用未能退出，已取消更新。' }
    $updateParent.Dispose()
    $updateParentExited = $true
    foreach ($updateFile in $updateFiles) {
        $updateSource = Join-Path (Join-Path $updateStage 'payload') $updateFile.name
        $updateTarget = Join-Path $updateRoot $updateFile.name
        $updateBackup = Join-Path (Join-Path $updateStage 'backup') $updateFile.name
        Assert-UpdatePath $updateTarget
        $updateExisted = Test-Path -LiteralPath $updateTarget
        if ($updateExisted) {
            $null = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($updateBackup))
            Copy-Item -LiteralPath $updateTarget -Destination $updateBackup
        }
        $null = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($updateTarget))
        # 同卷移动，运行中的其他实例锁定 EXE 时会失败并回滚，不强杀其他实例。
        if ($updateExisted) { Invoke-UpdateFileOperation { Remove-Item -LiteralPath $updateTarget -Force } }
        $updateChanged.Add(@{ target = $updateTarget; backup = $updateBackup; existed = $updateExisted })
        Move-Item -LiteralPath $updateSource -Destination $updateTarget
    }
    $updateChild = Start-UpdatedApplication $true
    $updateDeadline = [DateTime]::UtcNow.AddSeconds(30)
    $updateHealthy = Join-Path $updateStage 'healthy'
    while (-not (Test-Path -LiteralPath $updateHealthy)) {
        if ($updateChild.HasExited -or [DateTime]::UtcNow -gt $updateDeadline) { throw '新版启动失败，已恢复原版本。' }
        Start-Sleep -Milliseconds 100
    }
    if ([IO.File]::ReadAllText($updateHealthy) -cne $updatePlan.release.tag_name.Substring(1)) { throw '新版启动版本不符，已恢复原版本。' }
    # 仅清理本次应用目录下的暂存子目录。
    try {
        Assert-UpdatePath $updateStage
        Remove-Item -LiteralPath $updateStage -Recurse -Force
    } catch {
        # 清理失败不撤销已经健康启动的版本，保留日志供手动处理。
        try { [IO.File]::WriteAllText((Join-Path $updateStage 'error.txt'), "更新成功，暂存清理失败：$($_.Exception.Message)") } catch { }
    }
} catch {
    $updateFailure = $_.Exception.Message
    if ($null -ne $updateChild) {
        if (-not $updateChild.HasExited) { $updateChild.Kill() }
        $updateChild.WaitForExit()
        $updateChild.Dispose()
    }
    try {
        for ($updateIndex = $updateChanged.Count - 1; $updateIndex -ge 0; $updateIndex--) {
            $updateFile = $updateChanged[$updateIndex]
            Assert-UpdatePath $updateFile.target
            if ($updateFile.existed) { Invoke-UpdateFileOperation { Copy-Item -LiteralPath $updateFile.backup -Destination $updateFile.target -Force } }
            elseif (Test-Path -LiteralPath $updateFile.target) { Remove-Item -LiteralPath $updateFile.target -Force }
        }
        if ($updateParentExited) {
            $env:OMNITRIGGER_UPDATE_ERROR = "更新失败：$updateFailure"
            $null = Start-UpdatedApplication $false
        }
    } catch { $updateFailure += "；恢复失败：$($_.Exception.Message)。备份保留在 $updateStage" }
    if (Test-Path -LiteralPath $updateStage) { [IO.File]::WriteAllText((Join-Path $updateStage 'error.txt'), $updateFailure) }
    exit 1
}
