#requires -Version 7.0
<#
.SYNOPSIS
构建应用并返回本次 Cargo 回执中的 EXE 绝对路径。
#>
$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
try {
    $messages = & cargo build -p omnitrigger-app --locked --message-format=json-render-diagnostics |
        ForEach-Object { $_ | ConvertFrom-Json }
    if ($LASTEXITCODE -ne 0) { throw 'OmniTrigger 构建失败，请查看上述错误。' }

    $artifacts = @($messages | Where-Object {
        $_.reason -eq 'compiler-artifact' -and
        $_.target.name -eq 'omnitrigger' -and
        $_.target.kind -contains 'bin' -and
        $_.executable
    })
    if ($artifacts.Count -ne 1) { throw 'Cargo 未返回唯一的 OmniTrigger 可执行文件。' }
    $binaryPath = [string]$artifacts[0].executable
    if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
        throw "未找到本次构建产物：$binaryPath"
    }
    $binaryPath
} finally {
    Pop-Location
}
