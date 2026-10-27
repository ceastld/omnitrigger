#requires -Version 7.0
<#
.SYNOPSIS
运行与 Windows CI 相同的检查；仅在指定 -SmokeTest 时安装并卸载真实 Hook。
#>
[CmdletBinding()]
param([switch]$SmokeTest)

$ErrorActionPreference = 'Stop'

function Invoke-Cargo {
    param([string[]]$CargoArguments)
    & cargo @CargoArguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($CargoArguments -join ' ') 失败（退出码 $LASTEXITCODE）。"
    }
}

function Invoke-AppCheck {
    param([string]$BinaryPath, [string[]]$AppArguments)
    # 显式等待 GUI 子系统程序退出，并按参数数组传递含空格或中文的路径。
    $process = [System.Diagnostics.Process]::new()
    try {
        $process.StartInfo.FileName = $BinaryPath
        $process.StartInfo.UseShellExecute = $false
        $process.StartInfo.CreateNoWindow = $true
        $process.StartInfo.RedirectStandardOutput = $true
        $process.StartInfo.RedirectStandardError = $true
        foreach ($argument in $AppArguments) { $process.StartInfo.ArgumentList.Add($argument) }
        $null = $process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "检查超时：$($AppArguments -join ' ')"
        }
        if ($stdout.Result) { Write-Host $stdout.Result.TrimEnd() }
        if ($stderr.Result) { Write-Host $stderr.Result.TrimEnd() }
        if ($process.ExitCode -ne 0) {
            throw "应用检查失败（退出码 $($process.ExitCode)）：$($AppArguments -join ' ')"
        }
    } finally {
        $process.Dispose()
    }
}

if (-not $IsWindows) {
    throw '完整检查需要 Windows；其他平台可运行 cargo test -p omnitrigger-core --locked。'
}

Push-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
try {
    Invoke-Cargo -CargoArguments @('fmt', '--all', '--check')
    Invoke-Cargo -CargoArguments @('test', '--workspace', '--locked')
    Invoke-Cargo -CargoArguments @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')
    $binaryPath = & (Join-Path $PSScriptRoot 'build-app.ps1')

    foreach ($example in Get-ChildItem -LiteralPath examples -Filter '*.json' -File | Sort-Object Name) {
        Invoke-AppCheck -BinaryPath $binaryPath -AppArguments @('--config', $example.FullName, '--check-config')
    }
    if ($SmokeTest) {
        Invoke-AppCheck -BinaryPath $binaryPath -AppArguments @('--smoke-test')
    }
    Write-Host '格式、测试、Clippy、应用构建和示例配置检查通过。'
} finally {
    Pop-Location
}
