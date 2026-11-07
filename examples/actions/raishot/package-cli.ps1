param(
    [ValidateSet('prepare', 'verify')][string]$Operation = 'prepare',
    [string]$PackageDirectory = ''
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
$ProgressPreference = 'SilentlyContinue'
try {
    . (Join-Path $PSScriptRoot 'package.ps1')
    $result = Resolve-RaiShotPackage -FullVerify:($Operation -ceq 'verify') -Directory $PackageDirectory
    [Console]::WriteLine(($result | ConvertTo-Json -Compress -Depth 4))
    exit 0
} catch {
    # 不回传任意外部工具输出；失败不启动 RaiShot，也不自动重复安装。
    [Console]::WriteLine('{"error":"raishot_package_failed"}')
    exit 1
}
