# 只解包到本次新建的 staging/payload，不启动包内程序。兼容 Windows PowerShell 5.1。
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = $env:OMNITRIGGER_SCRIPT_ARCHIVE
$destination = $env:OMNITRIGGER_SCRIPT_PAYLOAD
if (-not $archive -or -not $destination -or (Test-Path -LiteralPath $destination)) { throw '解包参数无效或目标已存在。' }
$stream = [IO.File]::OpenRead($archive)
$zip = New-Object IO.Compression.ZipArchive($stream, [IO.Compression.ZipArchiveMode]::Read, $false)
try {
    if ($zip.Entries.Count -lt 6 -or $zip.Entries.Count -gt 1024) { throw '包内项目数量超限。' }
    $names = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $total = 0L
    foreach ($entry in $zip.Entries) {
        $name = $entry.FullName
        if (-not $name -or $name.Length -gt 240 -or $name.Contains('\') -or $name -match '[^A-Za-z0-9_./+-]' -or $name.StartsWith('/') -or -not $names.Add($name.TrimEnd('/'))) { throw '包内路径无效或重复。' }
        foreach ($part in $name.TrimEnd('/').Split('/')) {
            if (-not $part -or $part -eq '.' -or $part -eq '..' -or $part.EndsWith('.') -or $part -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)') { throw '包内路径包含非法组件。' }
        }
        if ((($entry.ExternalAttributes -shr 16) -band 0xf000) -eq 0xa000 -or ($entry.ExternalAttributes -band 0x400) -ne 0) { throw '包内不允许链接或重解析点。' }
        if ($entry.Length -lt 0 -or $entry.Length -gt 16MB) { throw '单文件大小超限。' }
        $total += $entry.Length
        if ($total -gt 64MB) { throw '包内总大小超限。' }
        $relative = $name.TrimEnd('/')
        if ($relative -notin @('manifest.json','host.exe','LICENSE','README.txt','THIRD_PARTY_NOTICES.md','sdk','sdk/api.d.ts','licenses') -and -not $relative.StartsWith('licenses/', [StringComparison]::Ordinal)) { throw '包内包含不支持的文件布局。' }
    }
    if (-not $names.Contains('manifest.json') -or -not $names.Contains('host.exe')) { throw '包内缺少清单或宿主。' }
    [IO.Directory]::CreateDirectory($destination) | Out-Null
    $root = [IO.Path]::GetFullPath($destination) + [IO.Path]::DirectorySeparatorChar
    foreach ($entry in $zip.Entries) {
        $target = [IO.Path]::GetFullPath((Join-Path $destination $entry.FullName.Replace('/',[IO.Path]::DirectorySeparatorChar)))
        if (-not $target.StartsWith($root,[StringComparison]::OrdinalIgnoreCase)) { throw '包内路径超出解包目录。' }
        if ($entry.FullName.EndsWith('/')) { [IO.Directory]::CreateDirectory($target) | Out-Null; continue }
        [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target)) | Out-Null
        $input = $entry.Open()
        $output = [IO.File]::Open($target,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        try {
            $buffer = New-Object byte[] 65536
            $written = 0L
            while (($count = $input.Read($buffer,0,$buffer.Length)) -gt 0) {
                $written += $count
                if ($written -gt $entry.Length -or $written -gt 16MB) { throw '实际解压大小超过声明。' }
                $output.Write($buffer,0,$count)
            }
            if ($written -ne $entry.Length) { throw '实际解压大小与声明不符。' }
        } finally { $input.Dispose(); $output.Dispose() }
    }
} finally { $zip.Dispose(); $stream.Dispose() }
