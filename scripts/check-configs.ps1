#requires -Version 7.0
<#
.SYNOPSIS
使用指定的本次构建 EXE 只读验证所有公开 JSON 配置示例。
#>
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$BinaryPath)

$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw '应用配置检查需要 Windows。' }
$binary = Get-Item -LiteralPath $BinaryPath -ErrorAction Stop
if ($binary.PSIsContainer -or $binary.Extension -ne '.exe') { throw '请指定应用 EXE 文件。' }
if (([int]$binary.Attributes -band (0x1000 -bor 0x40000 -bor 0x400000 -bor 0x400)) -ne 0) {
    throw '拒绝运行云端占位文件或链接。'
}
$exampleRoot = Join-Path (Split-Path -Parent $PSScriptRoot) 'examples'
foreach ($example in Get-ChildItem -LiteralPath $exampleRoot -Filter '*.json' -File | Sort-Object Name) {
    if (([int]$example.Attributes -band (0x1000 -bor 0x40000 -bor 0x400000 -bor 0x400)) -ne 0) {
        throw "拒绝读取云端占位示例或链接：$($example.Name)"
    }
    $process = [Diagnostics.Process]::new()
    try {
        $process.StartInfo.FileName = $binary.FullName
        $process.StartInfo.UseShellExecute = $false
        $process.StartInfo.CreateNoWindow = $true
        $process.StartInfo.RedirectStandardOutput = $true
        $process.StartInfo.RedirectStandardError = $true
        foreach ($argument in @('--config', $example.FullName, '--check-config')) {
            $process.StartInfo.ArgumentList.Add($argument)
        }
        $null = $process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "配置检查超时：$($example.Name)"
        }
        if ($stdout.Result) { Write-Host $stdout.Result.TrimEnd() }
        if ($stderr.Result) { Write-Host $stderr.Result.TrimEnd() }
        if ($process.ExitCode -ne 0) {
            throw "配置检查失败（退出码 $($process.ExitCode)）：$($example.Name)"
        }
    } finally {
        $process.Dispose()
    }
}
