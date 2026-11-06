# Legacy获取叶：调用前组合共用plugin_install_transport.ps1；只写本次stage，不运行包内程序。
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$ProgressPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.Net.Http

function Send-LegacyEvent([hashtable]$Value) {
    [Console]::WriteLine(($Value | ConvertTo-Json -Compress -Depth 4))
}

function Assert-LegacyRelative([string]$Name) {
    if (-not $Name -or $Name.Length -gt 240 -or $Name.Contains('\') -or
        $Name -match '[^A-Za-z0-9_./+-]' -or $Name.StartsWith('/') -or $Name.EndsWith('/')) {
        throw '制品路径无效。'
    }
    foreach ($part in $Name.Split('/')) {
        if (-not $part -or $part -eq '.' -or $part -eq '..' -or $part.EndsWith('.') -or
            $part -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)') { throw '制品路径组件无效。' }
    }
}

try {
    $legacyStage = [IO.Path]::GetFullPath($env:OMNITRIGGER_INSTALL_STAGE)
    Assert-PlainPath $legacyStage
    if (-not [IO.Directory]::Exists($legacyStage) -or
        -not [IO.Path]::GetFileName($legacyStage).StartsWith('.install-legacy-', [StringComparison]::Ordinal)) {
        throw '宿主暂存目录无效。'
    }
    $legacyExpectedBytes = [long]$env:OMNITRIGGER_INSTALL_BYTES
    $legacyHash = $env:OMNITRIGGER_INSTALL_SHA256
    $legacyManifestHash = $env:OMNITRIGGER_INSTALL_MANIFEST_SHA256
    if ($legacyExpectedBytes -le 0 -or $legacyExpectedBytes -gt 1GB -or
        $legacyHash -cnotmatch '^[a-f0-9]{64}$' -or $legacyManifestHash -cnotmatch '^[a-f0-9]{64}$') {
        throw '宿主制品描述无效。'
    }
    $legacyArchive = Join-Path $legacyStage 'archive.zip'
    if (-not [string]::IsNullOrWhiteSpace($env:OMNITRIGGER_INSTALL_ARCHIVE)) {
        if (-not [string]::IsNullOrWhiteSpace($env:OMNITRIGGER_INSTALL_URL)) { throw '制品来源不能混用。' }
        $legacySource = [IO.Path]::GetFullPath($env:OMNITRIGGER_INSTALL_ARCHIVE)
        Assert-PlainPath $legacySource
        $legacyItem = Get-Item -LiteralPath $legacySource
        if ($legacyItem.PSIsContainer -or $legacyItem.Length -ne $legacyExpectedBytes) { throw '本地制品大小不符。' }
        [IO.File]::Copy($legacySource, $legacyArchive, $false)
    } else {
        if ([string]::IsNullOrWhiteSpace($env:OMNITRIGGER_INSTALL_URL)) { throw '未提供宿主安装来源。' }
        Send-LegacyEvent @{ kind = 'progress'; message = '正在获取明确指定的宿主制品。' }
        if (-not (Download-Public $env:OMNITRIGGER_INSTALL_URL $legacyArchive $legacyExpectedBytes)) {
            throw '指定的宿主发布源不可用。'
        }
    }
    if ((Get-Item -LiteralPath $legacyArchive).Length -ne $legacyExpectedBytes -or
        (Get-Sha256 $legacyArchive) -cne $legacyHash) { throw '宿主归档大小或SHA-256不符。' }
    $legacyZip = [IO.Compression.ZipFile]::OpenRead($legacyArchive)
    try {
        if ($legacyZip.Entries.Count -lt 2 -or $legacyZip.Entries.Count -gt 1024) { throw '制品项目数量超限。' }
        $legacyEntries = New-Object 'Collections.Generic.Dictionary[string,IO.Compression.ZipArchiveEntry]' ([StringComparer]::OrdinalIgnoreCase)
        $legacyExpandedBytes = 0L
        foreach ($legacyEntry in $legacyZip.Entries) {
            Assert-LegacyRelative $legacyEntry.FullName
            if ($legacyEntries.ContainsKey($legacyEntry.FullName) -or
                (($legacyEntry.ExternalAttributes -shr 16) -band 0xf000) -eq 0xa000 -or
                ($legacyEntry.ExternalAttributes -band 0x400) -ne 0) { throw '制品不允许重复路径或链接。' }
            if ($legacyEntry.Length -lt 0 -or $legacyEntry.Length -gt 256MB) { throw '制品单文件超限。' }
            $legacyExpandedBytes += $legacyEntry.Length
            if ($legacyExpandedBytes -gt 1GB) { throw '制品展开大小超限。' }
            $legacyEntries.Add($legacyEntry.FullName, $legacyEntry)
        }
        if (-not $legacyEntries.ContainsKey('manifest.json')) { throw '制品缺少清单。' }
        $legacyManifestEntry = $legacyEntries['manifest.json']
        if ($legacyManifestEntry.FullName -cne 'manifest.json' -or $legacyManifestEntry.Length -gt 1MB) { throw '制品清单无效。' }
        $legacyManifestStream = $legacyManifestEntry.Open()
        $legacyManifestMemory = New-Object IO.MemoryStream
        try {
            $legacyManifestBuffer = New-Object byte[] 65536
            $legacyManifestWritten = 0L
            while (($legacyManifestRead = $legacyManifestStream.Read($legacyManifestBuffer, 0, $legacyManifestBuffer.Length)) -gt 0) {
                $legacyManifestWritten += $legacyManifestRead
                if ($legacyManifestWritten -gt $legacyManifestEntry.Length -or $legacyManifestWritten -gt 1MB) { throw '实际清单大小超限。' }
                $legacyManifestMemory.Write($legacyManifestBuffer, 0, $legacyManifestRead)
            }
            $legacyManifestBytes = $legacyManifestMemory.ToArray()
        } finally { $legacyManifestStream.Dispose(); $legacyManifestMemory.Dispose() }
        $legacyHasher = [Security.Cryptography.SHA256]::Create()
        try { $legacyManifestDigest = ([BitConverter]::ToString($legacyHasher.ComputeHash($legacyManifestBytes))).Replace('-', '').ToLowerInvariant() }
        finally { $legacyHasher.Dispose() }
        if ($legacyManifestBytes.Length -ne $legacyManifestEntry.Length -or $legacyManifestDigest -cne $legacyManifestHash) { throw '宿主清单SHA-256不符。' }
        $legacyManifest = [Text.Encoding]::UTF8.GetString($legacyManifestBytes).TrimStart([char]0xfeff) | ConvertFrom-Json
        if ($legacyManifest.schema_version -ne 1 -or $legacyManifest.architecture -cne 'windows-x64' -or
            [string]::IsNullOrWhiteSpace($legacyManifest.package_version) -or $legacyManifest.files.Count -lt 1 -or
            $legacyManifest.files.Count -gt 1023) { throw '宿主清单布局不兼容。' }
        $legacyExpected = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
        [void]$legacyExpected.Add('manifest.json')
        foreach ($legacyRecord in $legacyManifest.files) {
            Assert-LegacyRelative $legacyRecord.path
            if (-not $legacyExpected.Add($legacyRecord.path) -or -not $legacyEntries.ContainsKey($legacyRecord.path) -or
                $legacyEntries[$legacyRecord.path].FullName -cne $legacyRecord.path -or
                $legacyRecord.bytes -lt 0 -or $legacyRecord.bytes -ne $legacyEntries[$legacyRecord.path].Length -or
                $legacyRecord.sha256 -cnotmatch '^[a-f0-9]{64}$') { throw '清单文件记录与归档不符。' }
        }
        if ($legacyExpected.Count -ne $legacyEntries.Count) { throw '制品包含未登记文件。' }
        # 所有路径与闭集先校验，再创建payload。ZIP只接受文件项，目录由相对路径推导。
        $legacyPayload = Join-Path $legacyStage 'actions/legacy-host-entry'
        Assert-PlainPath $legacyPayload
        if (Test-Path -LiteralPath $legacyPayload) { throw '暂存制品已存在。' }
        [void][IO.Directory]::CreateDirectory($legacyPayload)
        foreach ($legacyEntry in $legacyZip.Entries) {
            $legacyTarget = Join-Path $legacyPayload $legacyEntry.FullName.Replace('/', [IO.Path]::DirectorySeparatorChar)
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($legacyTarget))
            $legacyInput = $legacyEntry.Open()
            $legacyOutput = [IO.File]::Open($legacyTarget, [IO.FileMode]::CreateNew)
            try {
                $legacyBuffer = New-Object byte[] 65536
                $legacyWritten = 0L
                while (($legacyCount = $legacyInput.Read($legacyBuffer, 0, $legacyBuffer.Length)) -gt 0) {
                    $legacyWritten += $legacyCount
                    if ($legacyWritten -gt $legacyEntry.Length) { throw '实际解压超过声明大小。' }
                    $legacyOutput.Write($legacyBuffer, 0, $legacyCount)
                }
                if ($legacyWritten -ne $legacyEntry.Length) { throw '实际解压大小不符。' }
            } finally { $legacyInput.Dispose(); $legacyOutput.Dispose() }
        }
        foreach ($legacyRecord in $legacyManifest.files) {
            if ((Get-Sha256 (Join-Path $legacyPayload $legacyRecord.path)) -cne $legacyRecord.sha256) { throw '宿主文件SHA-256不符。' }
        }
    } finally { $legacyZip.Dispose() }
    # Rust随后复用LocalLegacySource完整核验并登记；此结果只表示暂存完成。
    Send-LegacyEvent @{ kind = 'finished' }
    exit 0
} catch {
    Send-LegacyEvent @{ kind = 'failed'; message = ('宿主制品获取失败：' + $_.Exception.Message) }
    exit 1
}
