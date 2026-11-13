#requires -Version 7.0
<#
.SYNOPSIS
构建、检查并打包 Windows x64 安装器、便携 ZIP 与 SHA-256 清单。
#>
[CmdletBinding()]
param([ValidateSet('x86_64-pc-windows-msvc')][string]$Target = 'x86_64-pc-windows-msvc')

$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Windows 发行包必须在 Windows 上构建。' }
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
    $metadataText = & cargo metadata --no-deps --locked --format-version 1
    if ($LASTEXITCODE -ne 0) { throw '无法读取应用版本。' }
    $metadata = $metadataText | ConvertFrom-Json
    $app = @($metadata.packages | Where-Object name -eq 'omnitrigger-app')
    if ($app.Count -ne 1) { throw '无法确定唯一应用包。' }
    $version = [string]$app[0].version
    if ($version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') { throw '应用版本格式不支持。' }

    $messages = & cargo build -p omnitrigger-app --release --locked --target $Target --message-format=json-render-diagnostics |
        ForEach-Object { $_ | ConvertFrom-Json }
    if ($LASTEXITCODE -ne 0) { throw 'Windows Release 构建失败。' }
    $artifacts = @($messages | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'omnitrigger' -and $_.target.kind -contains 'bin' -and $_.executable })
    if ($artifacts.Count -ne 1) { throw 'Cargo 未返回唯一的 Release EXE。' }
    $binaryPath = [string]$artifacts[0].executable
    if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) { throw '本次 Release EXE 不存在。' }
    & (Join-Path $PSScriptRoot 'check-configs.ps1') -BinaryPath $binaryPath

    $distRoot = Join-Path $projectRoot 'dist'
    $null = New-Item -ItemType Directory -Path $distRoot -Force
    $packageName = "OmniTrigger-$version-windows-x86_64"
    $installerPath = Join-Path $distRoot "$packageName-setup.exe"
    $zipPath = Join-Path $distRoot "$packageName.zip"
    foreach ($output in @($installerPath, $zipPath)) {
        if (Test-Path -LiteralPath $output) { throw "发行文件已存在：$output" }
    }
    $stagingRoot = Join-Path $distRoot ".staging-$([Guid]::NewGuid().ToString('N'))"
    $null = New-Item -ItemType Directory -Path $stagingRoot
    try {
        Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $stagingRoot 'omnitrigger.exe')
        foreach ($name in @('LICENSE', 'THIRD_PARTY_NOTICES.md')) {
            Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination (Join-Path $stagingRoot $name)
        }
        $releaseNotes = Join-Path $projectRoot "docs/releases/v$version.md"
        if (-not (Test-Path -LiteralPath $releaseNotes -PathType Leaf)) { throw "缺少当前版本发行说明：docs/releases/v$version.md" }
        Copy-Item -LiteralPath $releaseNotes -Destination (Join-Path $stagingRoot 'RELEASE_NOTES.md')
        & (Join-Path $PSScriptRoot 'collect-licenses.ps1') -Target $Target -OutputDirectory (Join-Path $stagingRoot 'licenses')
        $readme = @"
OmniTrigger $version — Windows x64 早期预览版

安装完成后从开始菜单打开 OmniTrigger。便携版解压全部文件后运行 omnitrigger.exe。
建议使用 Windows 11 x64，Windows 10 尚未完整验证。
程序未进行代码签名。需要 Microsoft Visual C++ v14 Redistributable x64，缺少 VCRUNTIME140.dll 时请安装官方运行库：
https://aka.ms/vc14/vc_redist.x64.exe
官方运行库说明：https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist

日常配置保存在 %LOCALAPPDATA%\OmniTrigger，卸载或重装后保留。
开始菜单中的 OmniTrigger 测试使用单独保存的测试数据，默认关闭输入监听。便携版可使用 --test-workspace 打开。
应用支持键鼠组合、长按、快速拖拽、区域条件和本地动作。输入补发会改变时序，不能保证无损恢复。
可选截图插件需另行安装，不包含在本压缩包中；下载可能需要相应仓库访问权限。

完整更新说明与已知限制：RELEASE_NOTES.md
源代码、使用说明和安全报告：https://github.com/ceastld/omnitrigger
项目自身许可：LICENSE；依赖许可与版权：THIRD_PARTY_NOTICES.md、licenses/。
licenses/dependencies.json 记录锁定目标依赖、来源、许可文件与 SHA-256。
"@
        [IO.File]::WriteAllText((Join-Path $stagingRoot 'README.txt'), $readme, [Text.UTF8Encoding]::new($false))

        $compiler = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6/ISCC.exe'
        if (-not (Test-Path -LiteralPath $compiler)) { $compiler = (Get-Command ISCC.exe -ErrorAction Stop).Source }
        & $compiler /Qp "/DAppVersion=$version" "/DPayload=$stagingRoot" "/O$distRoot" (Join-Path $PSScriptRoot 'omnitrigger.iss')
        if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $installerPath -PathType Leaf)) { throw '安装器构建失败。' }
        [IO.Compression.ZipFile]::CreateFromDirectory($stagingRoot, $zipPath, [IO.Compression.CompressionLevel]::Optimal, $false)
        $lines = foreach ($file in @($zipPath, $installerPath)) {
            $hash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
            "$hash  $([IO.Path]::GetFileName($file))"
        }
        [IO.File]::WriteAllLines((Join-Path $distRoot 'SHA256SUMS.txt'), $lines, [Text.UTF8Encoding]::new($false))
        Write-Host ($lines -join "`n")
    } finally {
        # 仅清理本次创建且位于 dist 内的随机暂存目录。
        $resolvedStaging = [IO.Path]::GetFullPath($stagingRoot)
        $resolvedDist = [IO.Path]::GetFullPath($distRoot) + [IO.Path]::DirectorySeparatorChar
        if (-not $resolvedStaging.StartsWith($resolvedDist, [StringComparison]::OrdinalIgnoreCase)) { throw '拒绝清理 dist 以外的路径。' }
        if (Test-Path -LiteralPath $resolvedStaging) { Remove-Item -LiteralPath $resolvedStaging -Recurse -Force }
    }
} finally {
    Pop-Location
}
