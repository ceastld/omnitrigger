#requires -Version 7.0
<#
.SYNOPSIS
构建带 SDK、完整 Rust/QuickJS 通知和文件清单的本机开发包，不上传或发布。
#>
[CmdletBinding()]
param(
    [ValidateSet('quickjs')][string]$Engine = 'quickjs',
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or (& rustc -vV) -notcontains 'host: x86_64-pc-windows-msvc') { throw '开发包目前需要 Windows x64 MSVC。' }
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$output = [IO.Path]::GetFullPath($OutputDirectory)
function Assert-Plain([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    if ($current.StartsWith('\\')) { throw '开发包仅支持本地磁盘路径。' }
    if ($current -match '(?i)(^|[\\/])(OneDrive[^\\/]*|Dropbox|iCloud[^\\/]*)([\\/]|$)') { throw '开发包路径不能位于云盘。' }
    $ancestors = [Collections.Generic.List[string]]::new()
    while ($current) {
        $ancestors.Add($current)
        $parent = [IO.Directory]::GetParent($current)
        if ($null -eq $parent) { break }
        $current = $parent.FullName
    }
    $ancestors.Reverse()
    foreach ($candidate in $ancestors) {
        if (Test-Path -LiteralPath $candidate) {
            $item = Get-Item -LiteralPath $candidate -Force
            if (([long]$item.Attributes -band 0x441400) -ne 0) { throw "开发包路径包含链接或占位文件：$candidate" }
        }
    }
}
function Copy-Local([string]$Source, [string]$Destination) {
    Assert-Plain $Source
    $null = New-Item -ItemType Directory -Path (Split-Path -Parent $Destination) -Force
    Copy-Item -LiteralPath $Source -Destination $Destination
}
function Invoke-Host([string]$Executable, [string[]]$Arguments, [string]$InputJson) {
    $process = [Diagnostics.Process]::new()
    try {
        $process.StartInfo.FileName = $Executable
        $process.StartInfo.UseShellExecute = $false
        $process.StartInfo.CreateNoWindow = $true
        $process.StartInfo.RedirectStandardInput = $true
        $process.StartInfo.RedirectStandardOutput = $true
        $process.StartInfo.RedirectStandardError = $true
        $process.StartInfo.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
        $process.StartInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
        foreach ($argument in $Arguments) { $process.StartInfo.ArgumentList.Add($argument) }
        $null = $process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if ($InputJson) { $process.StandardInput.Write($InputJson) }
        $process.StandardInput.Close()
        if (-not $process.WaitForExit(10000)) { $process.Kill(); $process.WaitForExit(); throw '宿主试运行超过 10 秒。' }
        if ($process.ExitCode -ne 0 -or $stdout.Result.Length -gt 4096) { throw "宿主试运行失败：$($stderr.Result)" }
        $stdout.Result | ConvertFrom-Json
    } finally { $process.Dispose() }
}
Assert-Plain $output
if (Test-Path -LiteralPath $output) { throw '输出目录必须不存在，不覆盖旧开发包。' }
$null = New-Item -ItemType Directory -Path $output
$payload = Join-Path $output 'package'
$null = New-Item -ItemType Directory -Path $payload
Push-Location -LiteralPath $PSScriptRoot
try {
    $build = @('build', '--locked', '--no-default-features', '--features', $Engine, '--profile', 'small', '--target', 'x86_64-pc-windows-msvc', '--config', 'target.x86_64-pc-windows-msvc.rustflags=["-C","target-feature=+crt-static"]', '--message-format=json-render-diagnostics', '--target-dir', 'target/quickjs')
    $messages = & cargo @build | ForEach-Object { $_ | ConvertFrom-Json }
    if ($LASTEXITCODE -ne 0) { throw '独立宿主构建失败。' }
    $binary = @($messages | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'omnitrigger-js-probe' -and $_.executable })
    if ($binary.Count -ne 1) { throw 'Cargo 未返回唯一宿主。' }
    $hostPath = Join-Path $payload 'host.exe'
    Copy-Local $binary[0].executable $hostPath
    $description = Invoke-Host $hostPath @('--describe-runtime') ''
    if ($description.product -cne 'omnitrigger-script-host' -or $description.engine -cne $Engine -or $description.schema_version -ne 1 -or $description.protocol_schema -ne 1 -or $description.api_version -ne 1) { throw '宿主能力描述不兼容。' }
    $response = Invoke-Host $hostPath @() '{"script":"return {version:ctx.apiVersion,value:ctx.input.value};","input":{"value":"安装验证"}}'
    if ($response.schema_version -ne 1 -or $response.ok -ne $true -or $response.result.version -ne 1 -or $response.result.value -cne '安装验证') { throw '宿主实际 JS 调用验证失败。' }
    Copy-Local (Join-Path $repo 'crates/omnitrigger-actions/runtime/script-types.d.ts') (Join-Path $payload 'sdk/api.d.ts')
    Add-Content -LiteralPath (Join-Path $payload 'sdk/api.d.ts') -Value 'declare const ctx: ScriptContext;' -Encoding utf8NoBOM
    Copy-Local (Join-Path $repo 'LICENSE') (Join-Path $payload 'LICENSE')
    & (Join-Path $repo 'scripts/collect-licenses.ps1') -OutputDirectory (Join-Path $payload 'licenses') -ManifestPath (Join-Path $PSScriptRoot 'Cargo.toml') -PackageName 'omnitrigger-js-probe' -NoDefaultFeatures -Features $Engine
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $dumpbin = @(& $vswhere -latest -products '*' -find 'VC/Tools/MSVC/*/bin/Hostx64/x64/dumpbin.exe') | Sort-Object | Select-Object -Last 1
    if (-not $dumpbin) { throw '无法核验 PE 导入。' }
    $imports = @(& $dumpbin /nologo /dependents $hostPath | ForEach-Object { if ($_ -match '^\s*(\S+\.dll)\s*$') { $Matches[1] } })
    if ($LASTEXITCODE -ne 0 -or -not $imports.Count -or ($imports -match '^(VCRUNTIME|MSVCP|MSVCR)')) { throw '宿主仍依赖额外 VC 运行库，拒绝打包。' }
    $vs = @(& $vswhere -latest -products '*' -format json | ConvertFrom-Json)[0]
    [ordered]@{ crt = 'static'; imported_dlls = $imports; visual_studio_version = $vs.installationVersion; is_prerelease = [bool]$vs.isPrerelease; redistribution_review = 'required before any public binary release'; references = @('https://visualstudio.microsoft.com/license-terms/', 'https://learn.microsoft.com/en-us/visualstudio/releases/2026/redistribution', 'https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution') } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $payload 'licenses/msvc-build.json') -Encoding utf8NoBOM
    @'
# 第三方许可与构建记录

项目自身代码及 SDK 使用 MIT，见 LICENSE。QuickJS-NG 与 rquickjs 包装层保留其 MIT 许可和版权。
完整 Cargo 可达依赖、构建依赖、Rust 标准库通知、原始声明及补充来源见 licenses/dependencies.json 与对应文件；本项目的 MIT 不覆盖第三方内容。
本包仅用于本机开发验证，没有发布或上传。静态 MSVC CRT 的构建记录见 licenses/msvc-build.json，公开分发前仍须按实际工具链和适用许可完成再分发复核。
'@ | Set-Content -LiteralPath (Join-Path $payload 'THIRD_PARTY_NOTICES.md') -Encoding utf8NoBOM
    'OmniTrigger 独立脚本宿主本机开发包。host.exe 按动作调用启动，不安装常驻进程。sdk/api.d.ts 提供代码提示。此包不创建触发绑定，不修改用户动作。通过开发安装接口验证后使用；不是已发布 Release。' | Set-Content -LiteralPath (Join-Path $payload 'README.txt') -Encoding utf8NoBOM
    $source = (& git -C $repo rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $source -cnotmatch '^[0-9a-f]{40}$') { throw '无法记录源码提交。' }
    $dirty = @(& git -C $repo status --porcelain --untracked-files=normal -- tools/js-engine-probe crates/omnitrigger-actions/runtime crates/omnitrigger-windows crates/omnitrigger-core scripts/collect-licenses.ps1 scripts/license-overrides LICENSE).Count -gt 0
    $files = @(Get-ChildItem -LiteralPath $payload -Recurse -File | Sort-Object FullName | ForEach-Object {
        Assert-Plain $_.FullName
        [ordered]@{ path = [IO.Path]::GetRelativePath($payload, $_.FullName).Replace('\','/'); size = $_.Length; sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    $manifest = [ordered]@{ schema_version = 1; product = 'omnitrigger-script-host'; version = $description.version; engine = $Engine; target = 'windows-x86_64'; api_version = 1; protocol_schema = 1; distribution = 'development'; source_revision = $source; source_dirty = $dirty; files = $files }
    $manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $payload 'manifest.json') -Encoding utf8NoBOM
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = Join-Path $output 'script-runtime.zip'
    [IO.Compression.ZipFile]::CreateFromDirectory($payload, $zip, [IO.Compression.CompressionLevel]::Optimal, $false)
    $result = [ordered]@{ schema_version = 1; archive = $zip; sha256 = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant(); size = (Get-Item -LiteralPath $zip).Length; engine = $Engine; version = $description.version; distribution = 'development'; source_revision = $source; source_dirty = $dirty; protocol_and_JS_verified = $true }
    $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'result.json') -Encoding utf8NoBOM
    $result | ConvertTo-Json -Compress
} finally { Pop-Location }
