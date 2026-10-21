$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    & cargo build -p omnitrigger-app
    if ($LASTEXITCODE -ne 0) { throw 'OmniTrigger 构建失败，请查看上述错误。' }
    $binaryPath = Join-Path $PSScriptRoot 'target\debug\omnitrigger.exe'
    if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) { throw "未找到构建产物：$binaryPath" }
    Start-Process -FilePath $binaryPath -WorkingDirectory $PSScriptRoot
} finally {
    Pop-Location
}
