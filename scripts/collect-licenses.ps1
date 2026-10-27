#requires -Version 7.0
<#
.SYNOPSIS
收集 Windows 目标可达依赖的许可、版权及通知，不在发行清单中泄露构建机路径。
#>
[CmdletBinding()]
param(
    [string]$Target = 'x86_64-pc-windows-msvc',
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $outputRoot) { throw "许可输出目录必须尚不存在：$outputRoot" }

function Assert-LocalFile {
    param([string]$Path)
    $item = Get-Item -LiteralPath $Path -Force
    if (([int]$item.Attributes -band (0x1000 -bor 0x40000 -bor 0x400000 -bor 0x400)) -ne 0) {
        throw "拒绝读取云端占位文件或链接：$Path"
    }
    $item
}

function Get-NoticeFiles {
    param([string]$Root)
    $pending = [Collections.Generic.Queue[string]]::new()
    $pending.Enqueue($Root)
    while ($pending.Count) {
        foreach ($item in Get-ChildItem -LiteralPath ($pending.Dequeue()) -Force) {
            $null = Assert-LocalFile -Path $item.FullName
            if ($item.PSIsContainer) {
                if ($item.Name -notmatch '^(\.git|target|OneDrive.*|Dropbox|iCloud.*)$') {
                    $pending.Enqueue($item.FullName)
                }
            } elseif ($item.Name -match '(?i)^(licen[cs]e|copying|copyrights?|notices?|unlicense|patents?|authors?|credits|third[-_]party([-_](notices?|licen[cs]es?))?)([._-].*)?$' -or
                $item.DirectoryName -match '(?i)[\\/](licen[cs]es|notices)([\\/]|$)') {
                $item
            }
        }
    }
}

function Copy-Notice {
    param([string]$Source, [string]$Destination, [string]$RelativePath)
    $null = Assert-LocalFile -Path $Source
    $parent = Split-Path -Parent $Destination
    $null = New-Item -ItemType Directory -Path $parent -Force
    Copy-Item -LiteralPath $Source -Destination $Destination
    [ordered]@{
        path = $RelativePath.Replace('\', '/')
        sha256 = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

function Test-LicenseBody {
    param([IO.FileInfo]$File)
    # 某些发布包把符号链接展开成“../LICENSE-MIT”普通文本，不能当作许可全文。
    if ($File.Length -lt 200) { return $false }
    if ($File.Length -lt 1024) {
        $text = [IO.File]::ReadAllText($File.FullName).Trim()
        if ($text -match '^(?:\.\.?[\\/])+[^\r\n]+$') { return $false }
    }
    $true
}

Push-Location -LiteralPath $projectRoot
try {
    $metadataText = & cargo metadata --locked --format-version 1 --filter-platform $Target
    if ($LASTEXITCODE -ne 0) { throw '无法读取锁定目标的 Cargo 依赖图。' }
    $metadata = $metadataText | ConvertFrom-Json
    $packages = @{}; $nodes = @{}
    foreach ($package in $metadata.packages) { $packages[$package.id] = $package }
    foreach ($node in $metadata.resolve.nodes) { $nodes[$node.id] = $node }
    $app = @($metadata.packages | Where-Object name -eq 'omnitrigger-app')
    if ($app.Count -ne 1) { throw '无法确定应用包。' }

    $pending = [Collections.Generic.Queue[string]]::new()
    $pending.Enqueue($app[0].id)
    $reachable = [Collections.Generic.HashSet[string]]::new()
    while ($pending.Count) {
        $id = $pending.Dequeue()
        if ($reachable.Add($id)) {
            foreach ($dependency in $nodes[$id].deps) {
                if (@($dependency.dep_kinds | Where-Object { $_.kind -ne 'dev' }).Count) {
                    $pending.Enqueue($dependency.pkg)
                }
            }
        }
    }

    $overrideRoot = Join-Path $PSScriptRoot 'license-overrides'
    $overridePath = Join-Path $overrideRoot 'manifest.json'
    $null = Assert-LocalFile -Path $overridePath
    $overrides = Get-Content -LiteralPath $overridePath -Raw | ConvertFrom-Json
    $allowed = @('0BSD', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', 'BSL-1.0', 'CC0-1.0',
        'ISC', 'MIT', 'MIT-0', 'Unicode-3.0', 'Unlicense', 'Zlib', 'LLVM-exception')
    $entries = @()
    $null = New-Item -ItemType Directory -Path $outputRoot
    $dependencies = @($reachable | ForEach-Object { $packages[$_] } | Where-Object source | Sort-Object name, version)
    foreach ($package in $dependencies) {
        if (-not $package.license) { throw "$($package.name) 缺少许可证声明，需要人工核对。" }
        $identifiers = $package.license -replace '\b(AND|OR|WITH)\b|[()/]', ' ' -split '\s+' | Where-Object { $_ }
        foreach ($identifier in $identifiers) {
            if ($identifier -notin $allowed) { throw "$($package.name) 的许可证 $identifier 尚未审核。" }
        }
        $packageRoot = Split-Path -Parent $package.manifest_path
        $null = Assert-LocalFile -Path $packageRoot
        $packageFolder = "$($package.name)-$($package.version)"
        $packageOutput = Join-Path $outputRoot $packageFolder
        $noticeFiles = @(Get-NoticeFiles -Root $packageRoot)
        $licensePath = $null
        if ($package.license_file) {
            $licensePath = [IO.Path]::GetFullPath((Join-Path $packageRoot $package.license_file))
            if (-not $licensePath.StartsWith($packageRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
                throw "$($package.name) 的许可路径超出包目录。"
            }
            $noticeFiles += Get-Item -LiteralPath $licensePath
        }
        $copiedFiles = @()
        $licenseTextFiles = @()
        foreach ($file in @($noticeFiles | Sort-Object FullName -Unique)) {
            $relative = [IO.Path]::GetRelativePath($packageRoot, $file.FullName)
            $copiedFiles += Copy-Notice -Source $file.FullName -Destination (Join-Path $packageOutput $relative) -RelativePath "$packageFolder/$relative"
            if (($file.Name -match '(?i)^(licen[cs]e|copying|unlicense)([._-].*)?$' -or $file.FullName -eq $licensePath) -and
                (Test-LicenseBody -File $file)) {
                $licenseTextFiles += "$packageFolder/$($relative.Replace('\', '/'))"
            }
        }

        $supplement = @($overrides.packages | Where-Object { $_.name -eq $package.name -and $_.version -eq $package.version })
        if (-not $licenseTextFiles.Count -and $supplement.Count -ne 1) {
            throw "$($package.name) $($package.version) 没有许可全文或已审核的补充文件。"
        }
        $supplementSources = @()
        if ($supplement.Count -eq 1) {
            if ($supplement[0].license_expression -ne $package.license) { throw "$packageFolder 许可声明已变化，请重新审核。" }
            foreach ($file in $supplement[0].files) {
                $source = [IO.Path]::GetFullPath((Join-Path $overrideRoot $file.path))
                if (-not $source.StartsWith($overrideRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
                    throw '补充许可路径超出受控目录。'
                }
                $null = Assert-LocalFile -Path $source
                if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $file.sha256) { throw "补充许可哈希不一致：$($file.path)" }
                $relative = "supplement/$([IO.Path]::GetFileName($source))"
                $copiedFiles += Copy-Notice -Source $source -Destination (Join-Path $packageOutput $relative) -RelativePath "$packageFolder/$relative"
                if (Test-LicenseBody -File (Get-Item -LiteralPath $source)) {
                    $licenseTextFiles += "$packageFolder/$relative"
                }
                $supplementSources += [ordered]@{ file = "$packageFolder/$relative"; url = $file.source_url; note = $file.note }
            }
            # 保留包本身的原始授权声明与作者信息，补充文件不改写上游声明。
            foreach ($name in @('Cargo.toml.orig', 'Cargo.toml', 'README.md')) {
                $source = Join-Path $packageRoot $name
                if (Test-Path -LiteralPath $source -PathType Leaf) {
                    $copiedFiles += Copy-Notice -Source $source -Destination (Join-Path $packageOutput "declaration/$name") -RelativePath "$packageFolder/declaration/$name"
                }
            }
        }
        if (-not $licenseTextFiles.Count) { throw "$packageFolder 的文件仅含短声明或相对链接，缺少许可正文。" }
        $entries += [ordered]@{
            name = $package.name; version = $package.version; license_expression = $package.license
            authors = @($package.authors); repository = $package.repository; source = $package.source
            files = @($copiedFiles); license_text_files = @($licenseTextFiles); supplements = @($supplementSources)
        }
    }

    # Rust 标准库链接进 EXE，但不出现在 Cargo 包依赖图中。
    $sysroot = & rustc --print sysroot
    if ($LASTEXITCODE -ne 0) { throw '无法读取 Rust 标准库许可位置。' }
    $rustDocRoot = Join-Path $sysroot 'share/doc/rust'
    if (-not (Test-Path -LiteralPath (Join-Path $rustDocRoot 'COPYRIGHT-library.html') -PathType Leaf) -or
        -not (Test-Path -LiteralPath (Join-Path $rustDocRoot 'licenses') -PathType Container)) {
        throw 'Rust 标准库许可文件不完整；请为当前工具链安装 rust-docs（rustup component add rust-docs）。'
    }
    $rustOutput = Join-Path $outputRoot 'rust-standard-library'
    $rustFiles = @()
    $rustFiles += Copy-Notice -Source (Join-Path $rustDocRoot 'COPYRIGHT-library.html') -Destination (Join-Path $rustOutput 'COPYRIGHT-library.html') -RelativePath 'rust-standard-library/COPYRIGHT-library.html'
    foreach ($file in Get-NoticeFiles -Root (Join-Path $rustDocRoot 'licenses')) {
        $relative = [IO.Path]::GetRelativePath($rustDocRoot, $file.FullName)
        $rustFiles += Copy-Notice -Source $file.FullName -Destination (Join-Path $rustOutput $relative) -RelativePath "rust-standard-library/$relative"
    }
    # 此目录内的 SPDX 文件名不以 LICENSE 开头，因此一并复制固定工具链提供的正文。
    foreach ($file in Get-ChildItem -LiteralPath (Join-Path $rustDocRoot 'licenses') -File) {
        $relative = "rust-standard-library/licenses/$($file.Name)"
        if ($relative -notin @($rustFiles.path)) {
            $rustFiles += Copy-Notice -Source $file.FullName -Destination (Join-Path $outputRoot $relative) -RelativePath $relative
        }
    }
    $rustVersion = & rustc --version
    [ordered]@{
        schema_version = 1; target = $Target; dependency_scope = 'application reachable normal and build dependencies; excludes dev-only edges'
        packages = @($entries); rust_standard_library = [ordered]@{ toolchain = $rustVersion; files = @($rustFiles) }
    } | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $outputRoot 'dependencies.json') -Encoding utf8NoBOM
    Write-Host "已收集 $($entries.Count) 个目标依赖及 Rust 标准库的许可与通知。"
} finally {
    Pop-Location
}
