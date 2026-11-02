# 更新下载器与替换器共用的路径、网络和 ZIP 边界；不执行包内脚本。
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
Add-Type -AssemblyName System.Net.Http
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Assert-UpdatePath([string]$Path) {
    $updateCurrent = [IO.Path]::GetFullPath($Path)
    if ($updateCurrent -match '(?i)(^|[\\/])(OneDrive[^\\/]*|Dropbox|iCloud[^\\/]*)([\\/]|$)') { throw '更新目录不能位于云盘同步目录。' }
    $updateAncestors = New-Object 'Collections.Generic.Stack[string]'
    while ($updateCurrent) {
        $updateAncestors.Push($updateCurrent)
        $updateParent = [IO.Directory]::GetParent($updateCurrent)
        if ($null -eq $updateParent) { break }
        $updateCurrent = $updateParent.FullName
    }
    foreach ($updateCurrent in $updateAncestors) {
        if (Test-Path -LiteralPath $updateCurrent) {
            $updateItem = Get-Item -LiteralPath $updateCurrent -Force
            if (([long]$updateItem.Attributes -band (0x400 -bor 0x1000 -bor 0x40000 -bor 0x400000)) -ne 0) { throw '更新目录包含链接或云端占位文件。' }
        }
    }
}

function Write-UpdateEvent([hashtable]$Value) { [Console]::WriteLine(($Value | ConvertTo-Json -Compress -Depth 16)) }
function Write-UpdateProgress([string]$Message) { Write-UpdateEvent @{ kind = 'progress'; message = $Message } }

function Get-UpdateHash([string]$Path) {
    $updateHashStream = [IO.File]::OpenRead($Path)
    $updateHasher = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($updateHasher.ComputeHash($updateHashStream))).Replace('-', '') }
    finally { $updateHasher.Dispose(); $updateHashStream.Dispose() }
}

function Get-UpdateFile([string]$Url, [string]$Destination, [long]$Limit, [long]$Expected = 0) {
    $updateHandler = New-Object Net.Http.HttpClientHandler
    $updateHandler.AllowAutoRedirect = $false
    $updateClient = New-Object Net.Http.HttpClient($updateHandler)
    $updateClient.Timeout = [TimeSpan]::FromSeconds(30)
    $updateClient.DefaultRequestHeaders.UserAgent.ParseAdd('OmniTrigger-Updater')
    $updateResponse = $null
    try {
        $updateUri = [uri]$Url
        for ($updateRedirect = 0; $updateRedirect -lt 6; $updateRedirect++) {
            if ($updateUri.Scheme -cne 'https' -or -not $updateUri.IsDefaultPort -or $updateUri.UserInfo -or
                $updateUri.DnsSafeHost -notin @('api.github.com', 'github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com')) { throw '更新源返回了不受信任的下载地址。' }
            $updateResponse = $updateClient.GetAsync($updateUri, [Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
            if ([int]$updateResponse.StatusCode -in @(301, 302, 303, 307, 308)) {
                if ($null -eq $updateResponse.Headers.Location) { throw '更新源返回了无效重定向。' }
                $updateUri = New-Object uri($updateUri, $updateResponse.Headers.Location)
                $updateResponse.Dispose(); $updateResponse = $null
                continue
            }
            if ([int]$updateResponse.StatusCode -ne 200) { throw "无法访问更新源（HTTP $([int]$updateResponse.StatusCode)），请检查网络或稍后重试。" }
            $updateLength = $updateResponse.Content.Headers.ContentLength
            if ($null -ne $updateLength -and ($updateLength -gt $Limit -or ($Expected -gt 0 -and $updateLength -ne $Expected))) { throw '更新文件大小与发布信息不符。' }
            $updateInput = $updateResponse.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
            $updateOutput = [IO.File]::Open($Destination, [IO.FileMode]::CreateNew)
            try {
                $updateBuffer = New-Object byte[] 65536
                $updateTotal = 0L; $updatePercent = -1
                $updateDeadline = [DateTime]::UtcNow.AddMinutes(10)
                while ($true) {
                    if ([DateTime]::UtcNow -gt $updateDeadline) { throw '更新下载超时，请稍后重试。' }
                    $updateTask = $updateInput.ReadAsync($updateBuffer, 0, $updateBuffer.Length)
                    if (-not $updateTask.Wait(30000)) { throw '更新下载长时间没有响应，请稍后重试。' }
                    $updateRead = $updateTask.GetAwaiter().GetResult()
                    if ($updateRead -eq 0) { break }
                    $updateTotal += $updateRead
                    if ($updateTotal -gt $Limit -or ($Expected -gt 0 -and $updateTotal -gt $Expected)) { throw '更新文件超过发布大小。' }
                    $updateOutput.Write($updateBuffer, 0, $updateRead)
                    if ($Expected -gt 65536) {
                        $updateNext = [int][Math]::Floor(100.0 * $updateTotal / $Expected)
                        if ($updateNext -ne $updatePercent) { $updatePercent = $updateNext; Write-UpdateProgress "正在下载更新 $updatePercent%" }
                    }
                }
                if ($Expected -gt 0 -and $updateTotal -ne $Expected) { throw '更新文件下载不完整。' }
            } finally { $updateOutput.Dispose(); $updateInput.Dispose() }
            return
        }
        throw '更新下载重定向次数过多。'
    } finally {
        if ($null -ne $updateResponse) { $updateResponse.Dispose() }
        $updateClient.Dispose(); $updateHandler.Dispose()
    }
}

function Test-UpdateRelativePath([string]$Name) {
    # 不允许设备名、ADS、反斜杠、空段、尾随空格/点和大小写重复；仅接收发行文件。
    if ($Name -match '[:\\]' -or $Name.StartsWith('/') -or $Name.Length -gt 220) { return $false }
    foreach ($updateSegment in $Name.Split('/')) {
        if (-not $updateSegment -or $updateSegment -match '[. ]$' -or $updateSegment -match '^(?i:CON|PRN|AUX|NUL|COM[0-9]|LPT[0-9])(?:\.|$)') { return $false }
    }
    return ($Name -cin @('omnitrigger.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'README.txt', 'RELEASE_NOTES.md') -or $Name.StartsWith('licenses/', [StringComparison]::Ordinal))
}

function Expand-UpdateArchive([string]$Archive, [string]$Destination) {
    $updateZip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $updateNames = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
        $updateFiles = New-Object 'Collections.Generic.List[object]'
        $updateBytes = 0L
        if ($updateZip.Entries.Count -gt 10000) { throw '更新包包含过多文件。' }
        # 全量预检后才写盘，拒绝 ZIP 穿越、链接、重复路径和解压炸弹。
        foreach ($updateEntry in $updateZip.Entries) {
            $updateName = $updateEntry.FullName
            $updateType = ($updateEntry.ExternalAttributes -shr 16) -band 0xF000
            if ($updateType -eq 0xA000 -or ($updateEntry.ExternalAttributes -band 0x400) -ne 0) { throw '更新包不能包含链接。' }
            if ($updateName.EndsWith('/')) {
                if ($updateName -cne 'licenses/' -and -not (Test-UpdateRelativePath ($updateName.TrimEnd('/') + '/directory'))) { throw '更新包包含无效目录。' }
                continue
            }
            if (-not (Test-UpdateRelativePath $updateName) -or -not $updateNames.Add($updateName)) { throw '更新包包含不安全或重复的路径。' }
            $updateBytes += $updateEntry.Length
            if ($updateEntry.Length -gt 512MB -or $updateBytes -gt 1GB) { throw '更新包解压大小超限。' }
        }
        foreach ($updateRequired in @('omnitrigger.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.md')) {
            if (-not $updateNames.Contains($updateRequired)) { throw "更新包缺少 $updateRequired。" }
        }
        $null = [IO.Directory]::CreateDirectory($Destination)
        foreach ($updateEntry in $updateZip.Entries) {
            if ($updateEntry.FullName.EndsWith('/')) { continue }
            $updateTarget = Join-Path $Destination $updateEntry.FullName
            Assert-UpdatePath $updateTarget
            $null = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($updateTarget))
            [IO.Compression.ZipFileExtensions]::ExtractToFile($updateEntry, $updateTarget, $false)
            $updateFiles.Add(@{ name = $updateEntry.FullName; sha256 = (Get-UpdateHash $updateTarget) })
        }
        return ,$updateFiles.ToArray()
    } finally { $updateZip.Dispose() }
}

function Confirm-UpdateHash([string]$Checksums, [string]$ArchiveName, [string]$Archive) {
    $updateHashes = @(Get-Content -LiteralPath $Checksums | Where-Object { $_ -cmatch ('^([0-9a-fA-F]{64}) [ *]' + [regex]::Escape($ArchiveName) + '$') })
    if ($updateHashes.Count -ne 1) { throw '发布清单缺少唯一的更新包 SHA-256。' }
    $updateExpected = $updateHashes[0].Substring(0, 64)
    if ((Get-UpdateHash $Archive) -ine $updateExpected) { throw '更新包 SHA-256 校验失败，已保留当前版本。' }
}
