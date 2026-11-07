# RaiShot 依赖模块：仅准备与校验便携包；不启动程序，不读写 RaiShot 用户数据。
# 由本动作包的 package-cli.ps1 调用。
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:RaiShotRelease = @{
    Version = '0.4.18'
    Url = 'https://github.com/ceastld/raishot/releases/download/v0.4.18/RaiShot.0.4.18.0.zip'
    Bytes = 29657178L
    Sha256 = 'a56bc118e55ea53c865987a7bd28db80395ce4aa61653f3e70ca412f07238490'
    ArchiveName = 'RaiShot.0.4.18.0.zip'
}
$script:RaiShotMembers = @('raishot.exe', 'raishot-gpui.exe', 'raishot-cli.exe',
    'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'msvcp140_1.dll',
    'onnxruntime.dll', 'onnxruntime_providers_shared.dll', 'LICENSE-ONNXRuntime.txt',
    'THIRD-PARTY-ONNXRuntime.txt', 'README.txt', 'LICENSE', 'THIRD_PARTY_NOTICES.md',
    'LICENSE-GPUI-APACHE.txt', 'LICENSE-OmniTrigger.txt', 'manifest.json')

function Assert-RaiShotPath([string]$Path) {
    if (-not [IO.Path]::IsPathRooted($Path) -or $Path.StartsWith('\\')) { throw '依赖必须位于本机绝对路径。' }
    $current = [IO.Path]::GetFullPath($Path)
    if ($current -match '(?i)(^|[\\/])(OneDrive[^\\/]*|Dropbox|iCloud[^\\/]*)([\\/]|$)') { throw '依赖目录不能位于云盘。' }
    $ancestors = New-Object 'Collections.Generic.Stack[string]'
    while ($current) {
        $ancestors.Push($current)
        $parent = [IO.Directory]::GetParent($current)
        if ($null -eq $parent) { break }
        $current = $parent.FullName
    }
    foreach ($current in $ancestors) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (([long]$item.Attributes -band 0x441400) -ne 0) { throw '依赖路径包含链接或云端占位文件。' }
        }
    }
}

function Get-RaiShotHash([string]$Path) {
    Assert-RaiShotPath $Path
    $stream = [IO.File]::OpenRead($Path)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($hasher.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
    finally { $hasher.Dispose(); $stream.Dispose() }
}

function Get-RaiShotVersion([string]$Value) {
    if ($Value -cnotmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') { throw '依赖版本必须为三段整数。' }
    try { return [Version]::Parse($Value) } catch { throw '依赖版本无效。' }
}

function Assert-RaiShotName([string]$Name) {
    if ([string]::IsNullOrWhiteSpace($Name) -or $Name.Length -gt 255 -or $Name -in @('.', '..') -or
        $Name.IndexOfAny([IO.Path]::GetInvalidFileNameChars()) -ge 0 -or $Name -match '[\\/]' -or
        $Name.EndsWith('.') -or $Name.EndsWith(' ') -or $Name -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)') {
        throw '依赖清单包含无效文件名。'
    }
}

function Read-RaiShotManifest([string]$Directory, [switch]$FullVerify) {
    Assert-RaiShotPath $Directory
    $path = Join-Path $Directory 'manifest.json'
    Assert-RaiShotPath $path
    if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Get-Item -LiteralPath $path).Length -gt 65536) { throw '缺少有效的依赖清单。' }
    $manifest = [IO.File]::ReadAllText($path, (New-Object Text.UTF8Encoding($false, $true))) | ConvertFrom-Json
    foreach ($key in @('schema_version', 'package_name', 'package_version', 'architecture', 'files')) {
        if ($null -eq $manifest.PSObject.Properties[$key]) { throw '依赖清单字段不完整。' }
    }
    # 兼容 cea.rustshot 包名；官方 GitHub Release 工作流固定使用 rustshot。
    # 两者保留原始清单，不改写包名或清单内容。
    if (($manifest.schema_version -isnot [int] -and $manifest.schema_version -isnot [long]) -or
        $manifest.schema_version -ne 1 -or
        $manifest.package_name -isnot [string] -or @('cea.rustshot', 'rustshot') -cnotcontains $manifest.package_name -or
        $manifest.architecture -isnot [string] -or $manifest.architecture -cne 'windows-x64' -or $manifest.package_version -isnot [string] -or
        (Get-RaiShotVersion $manifest.package_version) -lt (Get-RaiShotVersion $script:RaiShotRelease.Version) -or
        $manifest.files -isnot [array] -or $manifest.files.Count -lt 4 -or $manifest.files.Count -gt 128) { throw '依赖包名、版本、平台或文件表无效。' }
    $seen = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($record in $manifest.files) {
        foreach ($key in @('path', 'bytes', 'sha256')) {
            if ($null -eq $record -or $null -eq $record.PSObject.Properties[$key]) { throw '依赖文件记录不完整。' }
        }
        if ($record.path -isnot [string] -or $record.sha256 -isnot [string]) { throw '依赖文件记录类型无效。' }
        Assert-RaiShotName $record.path
        if (-not $seen.Add($record.path) -or $record.path -ieq 'manifest.json' -or $record.path -ieq '.raishot-source.zip' -or
            ($record.bytes -isnot [int] -and $record.bytes -isnot [long]) -or $record.bytes -lt 0 -or
            $record.sha256 -cnotmatch '^[0-9a-fA-F]{64}$') { throw '依赖文件大小、哈希或名称无效。' }
        $file = Join-Path $Directory $record.path
        Assert-RaiShotPath $file
        if (-not (Test-Path -LiteralPath $file -PathType Leaf) -or (Get-Item -LiteralPath $file).Length -ne $record.bytes) { throw '依赖文件缺失或大小不符。' }
        if ($FullVerify -and (Get-RaiShotHash $file) -cne $record.sha256.ToLowerInvariant()) { throw '依赖文件 SHA-256 校验失败。' }
    }
    foreach ($name in @('raishot.exe', 'raishot-gpui.exe', 'raishot-cli.exe', 'vcruntime140.dll')) {
        if (-not $seen.Contains($name)) { throw '依赖缺少必要程序或运行库。' }
    }
    foreach ($key in @('runtime', 'ocr_runtime')) {
        if ($null -eq $manifest.PSObject.Properties[$key]) { continue }
        $runtime = $manifest.$key
        if ($null -eq $runtime -or $null -eq $runtime.PSObject.Properties['files'] -or $runtime.files -isnot [array]) { throw '运行库声明无效。' }
        foreach ($name in $runtime.files) {
            if ($name -isnot [string] -or -not $seen.Contains($name)) { throw '运行库文件未包含在校验清单中。' }
        }
    }
    return $manifest
}

function Open-RaiShotArchive([string]$Path) {
    Assert-RaiShotPath $Path
    if ((Get-Item -LiteralPath $Path).Length -ne $script:RaiShotRelease.Bytes -or
        (Get-RaiShotHash $Path) -cne $script:RaiShotRelease.Sha256) { throw '官方发布包的大小或 SHA-256 不符。' }
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        if ($zip.Entries.Count -ne $script:RaiShotMembers.Count) { throw '发布包文件数量不符。' }
        $seen = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
        $expanded = 0L
        foreach ($entry in $zip.Entries) {
            if (-not ($script:RaiShotMembers -ccontains $entry.FullName) -or -not $seen.Add($entry.FullName) -or
                (($entry.ExternalAttributes -shr 16) -band 61440) -eq 40960 -or
                ($entry.ExternalAttributes -band 0x400) -ne 0 -or $entry.Length -le 0) { throw '发布包含未知路径、链接、空文件或重复文件。' }
            $expanded += $entry.Length
            if ($expanded -gt 268435456) { throw '发布包展开大小超过限制。' }
        }
        if ($zip.GetEntry('manifest.json').Length -gt 65536) { throw '发布清单过大。' }
        return $zip
    } catch { $zip.Dispose(); throw }
}

function Assert-RaiShotArchiveManifest([string]$Directory, [string]$Archive) {
    $zip = Open-RaiShotArchive $Archive
    try {
        $stream = $zip.GetEntry('manifest.json').Open()
        $hasher = [Security.Cryptography.SHA256]::Create()
        try { $expected = ([BitConverter]::ToString($hasher.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
        finally { $hasher.Dispose(); $stream.Dispose() }
        if ((Get-RaiShotHash (Join-Path $Directory 'manifest.json')) -cne $expected) { throw '安装清单与已校验的官方 ZIP 不符。' }
    } finally { $zip.Dispose() }
}

function Receive-RaiShotArchive([string]$Destination) {
    # 官方源固定且有 SHA-256；不调用安装 EXE、gh 或任何下载内容。
    Add-Type -AssemblyName System.Net.Http
    $handler = New-Object Net.Http.HttpClientHandler
    $handler.AllowAutoRedirect = $false
    $client = New-Object Net.Http.HttpClient($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(40)
    $client.DefaultRequestHeaders.UserAgent.ParseAdd('OmniTrigger/0.1')
    $deadline = [DateTime]::UtcNow.AddSeconds(40)
    try {
        $uri = [uri]$script:RaiShotRelease.Url
        for ($redirect = 0; $redirect -lt 6; $redirect++) {
            if ($uri.Scheme -cne 'https' -or -not $uri.IsDefaultPort -or -not [string]::IsNullOrEmpty($uri.UserInfo) -or
                @('github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com') -notcontains $uri.DnsSafeHost) { throw '发布源重定向无效。' }
            $left = [Math]::Max(1, [int]($deadline - [DateTime]::UtcNow).TotalMilliseconds)
            $pending = $client.GetAsync($uri, [Net.Http.HttpCompletionOption]::ResponseHeadersRead)
            if (-not $pending.Wait($left)) { throw '下载发布包超时。' }
            $response = $pending.GetAwaiter().GetResult()
            try {
                if ([int]$response.StatusCode -in @(301, 302, 303, 307, 308)) {
                    if ($null -eq $response.Headers.Location) { throw '发布源缺少重定向地址。' }
                    $uri = New-Object uri($uri, $response.Headers.Location)
                    continue
                }
                if ([int]$response.StatusCode -ne 200 -or $response.Content.Headers.ContentLength -ne $script:RaiShotRelease.Bytes) { throw '官方发布包暂不可用或大小不符。' }
                $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
                $output = [IO.File]::Open($Destination, [IO.FileMode]::CreateNew)
                try {
                    $buffer = New-Object byte[] 65536
                    $total = 0L
                    while ($true) {
                        $left = [int]($deadline - [DateTime]::UtcNow).TotalMilliseconds
                        if ($left -le 0) { throw '下载发布包超时。' }
                        $pendingRead = $inputStream.ReadAsync($buffer, 0, $buffer.Length)
                        if (-not $pendingRead.Wait($left)) { throw '下载发布包超时。' }
                        $count = $pendingRead.GetAwaiter().GetResult()
                        if ($count -eq 0) { break }
                        $total += $count
                        if ($total -gt $script:RaiShotRelease.Bytes) { throw '发布包超出固定大小。' }
                        $output.Write($buffer, 0, $count)
                    }
                    if ($total -ne $script:RaiShotRelease.Bytes) { throw '发布包下载不完整。' }
                } finally { $output.Dispose(); $inputStream.Dispose() }
                return
            } finally { $response.Dispose() }
        }
        throw '发布源重定向次数过多。'
    } finally { $client.Dispose(); $handler.Dispose() }
}

function Install-RaiShotRelease([string]$Root, [string]$ArchivePath = '') {
    Assert-RaiShotPath $Root
    $destination = Join-Path $Root $script:RaiShotRelease.Version
    Assert-RaiShotPath $destination
    if (Test-Path -LiteralPath $destination) {
        Assert-RaiShotArchiveManifest $destination (Join-Path $destination '.raishot-source.zip')
        return (Read-RaiShotManifest $destination -FullVerify)
    }
    [void][IO.Directory]::CreateDirectory($Root)
    $stage = Join-Path $Root ('.install-' + [Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($stage)
    try {
        $archive = Join-Path $stage $script:RaiShotRelease.ArchiveName
        if ($ArchivePath) {
            Assert-RaiShotPath $ArchivePath
            if ((Get-Item -LiteralPath $ArchivePath).Length -ne $script:RaiShotRelease.Bytes) { throw '离线包大小不符。' }
            [IO.File]::Copy($ArchivePath, $archive, $false)
        } else { Receive-RaiShotArchive $archive }
        $zip = Open-RaiShotArchive $archive
        $payload = Join-Path $stage 'payload'
        [void][IO.Directory]::CreateDirectory($payload)
        try {
            foreach ($entry in $zip.Entries) {
                $source = $entry.Open()
                $output = [IO.File]::Open((Join-Path $payload $entry.FullName), [IO.FileMode]::CreateNew)
                try { $source.CopyTo($output) } finally { $output.Dispose(); $source.Dispose() }
            }
        } finally { $zip.Dispose() }
        $manifest = Read-RaiShotManifest $payload -FullVerify
        if ($manifest.package_version -cne $script:RaiShotRelease.Version) { throw '固定发布包的版本不符。' }
        $declared = @($manifest.files | ForEach-Object { $_.path }) + @('manifest.json')
        if ($declared.Count -ne $script:RaiShotMembers.Count) { throw '发布清单成员不完整。' }
        foreach ($name in $script:RaiShotMembers) {
            if ($declared -cnotcontains $name) { throw '发布清单缺少固定成员。' }
        }
        [IO.File]::Move($archive, (Join-Path $payload '.raishot-source.zip'))
        Assert-RaiShotPath $destination
        # 同版本目录已存在时 Move 失败；不会覆盖现有安装、配置或运行中的程序。
        [IO.Directory]::Move($payload, $destination)
        return $manifest
    } finally {
        # 只清理本次创建且仍在指定 Root 下的暂存目录。
        if ([IO.Directory]::GetParent($stage).FullName -ceq [IO.Path]::GetFullPath($Root)) {
            Assert-RaiShotPath $stage
            if (Test-Path -LiteralPath $stage -PathType Container) { [IO.Directory]::Delete($stage, $true) }
        }
    }
}

function Resolve-RaiShotPackage([switch]$FullVerify, [string]$Directory = '') {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or -not [Environment]::Is64BitOperatingSystem -or [IntPtr]::Size -ne 8) { throw 'RaiShot 需要 Windows x64 宿主。' }
    $root = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs\RaiShot'
    if ($Directory) { $directories = @([IO.Path]::GetFullPath($Directory)) }
    else {
        Assert-RaiShotPath $root
        $directories = @($root)
        if (Test-Path -LiteralPath $root -PathType Container) {
            $children = @(Get-ChildItem -LiteralPath $root -Directory -Force | Where-Object { $_.Name -cmatch '^[0-9]+\.[0-9]+\.[0-9]+$' } | Select-Object -First 65)
            if ($children.Count -gt 64) { throw '依赖版本目录过多，请显式选择依赖目录。' }
            $directories += @($children | ForEach-Object { $_.FullName })
        }
    }
    $best = $null
    foreach ($candidate in $directories) {
        try { $manifest = Read-RaiShotManifest $candidate }
        catch { if ($Directory) { throw }; continue }
        if ($null -eq $best -or (Get-RaiShotVersion $manifest.package_version) -gt (Get-RaiShotVersion $best.manifest.package_version)) {
            $best = @{ directory = $candidate; manifest = $manifest }
        }
    }
    $downloaded = $false
    if ($null -eq $best) {
        $manifest = Install-RaiShotRelease $root
        $best = @{ directory = (Join-Path $root $script:RaiShotRelease.Version); manifest = $manifest }
        $downloaded = $true
    }
    $authenticated = $downloaded
    if ($FullVerify) {
        $best.manifest = Read-RaiShotManifest $best.directory -FullVerify
        $archive = Join-Path $best.directory '.raishot-source.zip'
        Assert-RaiShotPath $archive
        if (Test-Path -LiteralPath $archive) {
            Assert-RaiShotArchiveManifest $best.directory $archive
            $authenticated = $true
        }
    }
    return [ordered]@{ directory = $best.directory; exe_path = (Join-Path $best.directory 'raishot.exe');
        version = $best.manifest.package_version; verification = $(if ($FullVerify -or $downloaded) { 'full' } else { 'size' });
        source = $(if ($downloaded) { 'downloaded' } else { 'existing' }); archive_authenticated = $authenticated;
        package_name = $best.manifest.package_name }
}
