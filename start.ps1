#requires -Version 7.0
$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    $binaryPath = & (Join-Path $PSScriptRoot 'scripts/build-app.ps1')
    Start-Process -FilePath $binaryPath -ArgumentList '--test-workspace' -WorkingDirectory $PSScriptRoot
} finally {
    Pop-Location
}
