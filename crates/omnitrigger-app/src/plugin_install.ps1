# 插件依赖安装器：由 Rust 后台线程启动。所有参数通过环境传入，不执行下载内容。
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$ProgressPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.Net.Http

function Send-InstallEvent([hashtable]$Value) {
    [Console]::WriteLine(($Value | ConvertTo-Json -Compress -Depth 4))
}

function Send-InstallProgress([string]$Message) {
    Send-InstallEvent @{ kind = 'progress'; message = $Message }
}

function Assert-PlainPath([string]$Path) {
    $installCurrent = [IO.Path]::GetFullPath($Path)
    if ($installCurrent -match '(?i)(^|[\\/])(OneDrive[^\\/]*|Dropbox|iCloud[^\\/]*)([\\/]|$)') {
        throw '插件目录不能位于云盘同步目录。'
    }
    $installAncestors = New-Object 'Collections.Generic.Stack[string]'
    while ($installCurrent) {
        $installAncestors.Push($installCurrent)
        $installParent = [IO.Directory]::GetParent($installCurrent)
        if ($null -eq $installParent) { break }
        $installCurrent = $installParent.FullName
    }
    # Stack 按根到叶的顺序枚举，先拒绝父级链接/云端目录，再访问子路径。
    foreach ($installCurrent in $installAncestors) {
        if (Test-Path -LiteralPath $installCurrent) {
            $installItem = Get-Item -LiteralPath $installCurrent -Force
            if (([long]$installItem.Attributes -band (0x400 -bor 0x1000 -bor 0x40000 -bor 0x400000)) -ne 0) {
                throw '插件目录包含链接、联接点或云端占位文件。'
            }
        }
    }
}

function Get-Sha256([string]$Path) {
    $installHashStream = [IO.File]::OpenRead($Path)
    $installHasher = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($installHasher.ComputeHash($installHashStream))).Replace('-', '').ToLowerInvariant() }
    finally { $installHasher.Dispose(); $installHashStream.Dispose() }
}

function Test-TrustedDownload([uri]$Uri) {
    return ($Uri.Scheme -ceq 'https' -and $Uri.IsDefaultPort -and [string]::IsNullOrEmpty($Uri.UserInfo) -and
        @('github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com') -contains $Uri.DnsSafeHost)
}

function Download-Public([string]$Url, [string]$Destination, [long]$ExpectedBytes) {
    $installHandler = New-Object Net.Http.HttpClientHandler
    $installHandler.AllowAutoRedirect = $false
    $installClient = New-Object Net.Http.HttpClient($installHandler)
    $installClient.Timeout = [TimeSpan]::FromSeconds(180)
    $installClient.DefaultRequestHeaders.UserAgent.ParseAdd('OmniTrigger/0.1')
    $installResponse = $null
    try {
        $installUri = [uri]$Url
        for ($installRedirect = 0; $installRedirect -lt 6; $installRedirect++) {
            if (-not (Test-TrustedDownload $installUri)) { throw '下载地址不属于已配置的 HTTPS 发布源。' }
            $installResponse = $installClient.GetAsync($installUri, [Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
            $installStatus = [int]$installResponse.StatusCode
            if ($installStatus -in @(301, 302, 303, 307, 308)) {
                $installLocation = $installResponse.Headers.Location
                if ($null -eq $installLocation) { throw '发布源返回无效重定向。' }
                $installUri = New-Object uri($installUri, $installLocation)
                $installResponse.Dispose()
                $installResponse = $null
                continue
            }
            if ($installStatus -ne 200) { return $false }
            if ($installResponse.Content.Headers.ContentLength -ne $ExpectedBytes) { throw '发布包大小与固定清单不符。' }
            $installInput = $installResponse.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
            $installOutput = [IO.File]::Open($Destination, [IO.FileMode]::CreateNew)
            try {
                $installBuffer = New-Object byte[] 65536
                $installTotal = 0L
                $installDeadline = [DateTime]::UtcNow.AddSeconds(180)
                while ($true) {
                    if ([DateTime]::UtcNow -gt $installDeadline) { throw '下载发布包超时，请稍后重试。' }
                    $installReadTask = $installInput.ReadAsync($installBuffer, 0, $installBuffer.Length)
                    if (-not $installReadTask.Wait(30000)) { throw '下载发布包长时间没有响应。' }
                    $installRead = $installReadTask.GetAwaiter().GetResult()
                    if ($installRead -eq 0) { break }
                    $installTotal += $installRead
                    if ($installTotal -gt $ExpectedBytes) { throw '发布包超过固定大小，已停止下载。' }
                    $installOutput.Write($installBuffer, 0, $installRead)
                }
                if ($installTotal -ne $ExpectedBytes) { throw '发布包下载不完整。' }
            } finally {
                $installOutput.Dispose()
                $installInput.Dispose()
            }
            return $true
        }
        throw '发布源重定向次数过多。'
    } finally {
        if ($null -ne $installResponse) { $installResponse.Dispose() }
        $installClient.Dispose()
        $installHandler.Dispose()
    }
}

function Download-WithGh([string]$Stage) {
    $installGh = Get-Command gh -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -eq $installGh) { throw '发布包暂不可匿名下载。请安装并登录 GitHub CLI 后重试，或使用本机已有的 RaiShot CLI。' }
    Send-InstallProgress '发布源需要访问权限，正在使用本机已登录的 GitHub CLI 下载。'
    $installInfo = New-Object Diagnostics.ProcessStartInfo
    $installInfo.FileName = $installGh.Source
    $installInfo.Arguments = 'release download v0.4.15 --repo ceastld/raishot --pattern RaiShot.0.4.15.0.zip --clobber'
    $installInfo.WorkingDirectory = $Stage
    $installInfo.UseShellExecute = $false
    $installInfo.CreateNoWindow = $true
    $installInfo.RedirectStandardOutput = $true
    $installInfo.RedirectStandardError = $true
    $installProcess = New-Object Diagnostics.Process
    $installProcess.StartInfo = $installInfo
    try {
        [void]$installProcess.Start()
        $installOut = $installProcess.StandardOutput.ReadToEndAsync()
        $installErr = $installProcess.StandardError.ReadToEndAsync()
        if (-not $installProcess.WaitForExit(180000)) {
            $installProcess.Kill()
            $installProcess.WaitForExit()
            throw '下载发布包超时，请稍后重试。'
        }
        if ($installProcess.ExitCode -ne 0) {
            throw '无法下载 RaiShot 发布包。请确认 GitHub CLI 已登录且具有发布源访问权限，或指定已有的 RaiShot CLI。'
        }
        # 不将认证工具原始输出写入 UI 或日志。
        [void]$installOut.GetAwaiter().GetResult()
        [void]$installErr.GetAwaiter().GetResult()
    } finally { $installProcess.Dispose() }
}

function Assert-InstalledFiles([string]$Directory, [string]$Version, [string]$ManifestHash) {
    Assert-PlainPath $Directory
    $installManifestPath = Join-Path $Directory 'manifest.json'
    Assert-PlainPath $installManifestPath
    if (-not (Test-Path -LiteralPath $installManifestPath -PathType Leaf) -or
        (Get-Item -LiteralPath $installManifestPath).Length -gt 65536 -or
        (Get-Sha256 $installManifestPath) -cne $ManifestHash) {
        throw '安装包的文件清单校验失败。'
    }
    $installManifest = [IO.File]::ReadAllText($installManifestPath) | ConvertFrom-Json
    if ($installManifest.schema_version -ne 1 -or $installManifest.package_version -cne $Version -or $installManifest.architecture -cne 'windows-x64') {
        throw '安装包版本或平台与插件要求不符。'
    }
    $installSeen = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($installRecord in $installManifest.files) {
        if (-not ($script:installAllowedFiles -ccontains $installRecord.path) -or -not $installSeen.Add($installRecord.path)) {
            throw '安装包文件清单包含未知或重复文件。'
        }
        $installFile = Join-Path $Directory $installRecord.path
        Assert-PlainPath $installFile
        if (-not (Test-Path -LiteralPath $installFile -PathType Leaf) -or
            (Get-Item -LiteralPath $installFile).Length -ne $installRecord.bytes -or
            (Get-Sha256 $installFile) -cne $installRecord.sha256) { throw '安装包文件不完整或内容已改变。' }
    }
    foreach ($installRequired in $script:installAllowedFiles) {
        if ($installRequired -cne 'manifest.json' -and -not $installSeen.Contains($installRequired)) { throw '安装包缺少必要程序、运行库或许可证。' }
    }
}

try {
    $installRoot = [IO.Path]::GetFullPath($env:OMNITRIGGER_INSTALL_ROOT)
    $installStage = [IO.Path]::GetFullPath($env:OMNITRIGGER_INSTALL_STAGE)
    $installVersion = $env:OMNITRIGGER_INSTALL_VERSION
    $installHash = $env:OMNITRIGGER_INSTALL_SHA256
    $installManifestHash = $env:OMNITRIGGER_INSTALL_MANIFEST_SHA256
    $installExpectedBytes = [long]$env:OMNITRIGGER_INSTALL_BYTES
    if ($installVersion -cnotmatch '^[0-9]+\.[0-9]+\.[0-9]+$' -or $installHash -cnotmatch '^[a-f0-9]{64}$' -or
        $installManifestHash -cnotmatch '^[a-f0-9]{64}$' -or $installExpectedBytes -le 0 -or $installExpectedBytes -gt 67108864) {
        throw '插件安装清单无效。'
    }
    if ([IO.Directory]::GetParent($installStage).FullName -cne $installRoot -or
        -not ([IO.Path]::GetFileName($installStage).StartsWith('.install-', [StringComparison]::Ordinal))) { throw '插件暂存目录无效。' }
    Assert-PlainPath $installRoot
    Assert-PlainPath $installStage
    $script:installAllowedFiles = @('raishot.exe', 'raishot-gpui.exe', 'raishot-cli.exe',
        'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'msvcp140_1.dll',
        'onnxruntime.dll', 'onnxruntime_providers_shared.dll', 'LICENSE-ONNXRuntime.txt',
        'THIRD-PARTY-ONNXRuntime.txt', 'README.txt', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'manifest.json')
    $installDestination = Join-Path $installRoot $installVersion
    if (Test-Path -LiteralPath $installDestination) {
        Send-InstallProgress '正在核验已安装的 RaiShot 和运行库。'
        Assert-InstalledFiles $installDestination $installVersion $installManifestHash
    } else {
        $installArchive = Join-Path $installStage 'RaiShot.0.4.15.0.zip'
        if (-not [string]::IsNullOrWhiteSpace($env:OMNITRIGGER_INSTALL_ARCHIVE)) {
            Send-InstallProgress '正在读取并核验本地 RaiShot 依赖包。'
            $installSourceArchive = [IO.Path]::GetFullPath($env:OMNITRIGGER_INSTALL_ARCHIVE)
            Assert-PlainPath $installSourceArchive
            if ((Get-Item -LiteralPath $installSourceArchive).Length -ne $installExpectedBytes) { throw '离线包大小与插件要求不符。' }
            [IO.File]::Copy($installSourceArchive, $installArchive, $false)
        } else {
            Send-InstallProgress '正在下载 RaiShot 和运行库（约 26 MiB）。'
            if (-not (Download-Public $env:OMNITRIGGER_INSTALL_URL $installArchive $installExpectedBytes)) { Download-WithGh $installStage }
        }
        if ((Get-Item -LiteralPath $installArchive).Length -ne $installExpectedBytes -or
            (Get-Sha256 $installArchive) -cne $installHash) { throw '发布包 SHA-256 或大小校验失败，原有安装已保留。' }
        Send-InstallProgress '发布包校验通过，正在安装程序、运行库和许可证。'
        $installPayload = Join-Path $installStage 'payload'
        [void][IO.Directory]::CreateDirectory($installPayload)
        $installZip = [IO.Compression.ZipFile]::OpenRead($installArchive)
        try {
            if ($installZip.Entries.Count -ne $script:installAllowedFiles.Count) { throw '发布包文件数量不符。' }
            $installZipSeen = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
            $installExpandedBytes = 0L
            foreach ($installEntry in $installZip.Entries) {
                # 当前固定官方包只包含这 15 个根目录文件，拒绝子目录、路径穿越、链接及重复名。
                if (-not ($script:installAllowedFiles -ccontains $installEntry.FullName) -or
                    -not $installZipSeen.Add($installEntry.FullName) -or
                    (($installEntry.ExternalAttributes -shr 16) -band 61440) -eq 40960) { throw '发布包含不允许的路径、链接或重复文件。' }
                $installExpandedBytes += $installEntry.Length
                if ($installExpandedBytes -gt 134217728 -or $installEntry.Length -le 0) { throw '发布包解压大小超出限制。' }
            }
            foreach ($installEntry in $installZip.Entries) {
                $installOutputPath = Join-Path $installPayload $installEntry.FullName
                $installEntryStream = $installEntry.Open()
                $installFileStream = [IO.File]::Open($installOutputPath, [IO.FileMode]::CreateNew)
                try { $installEntryStream.CopyTo($installFileStream) }
                finally { $installFileStream.Dispose(); $installEntryStream.Dispose() }
            }
        } finally { $installZip.Dispose() }
        Assert-InstalledFiles $installPayload $installVersion $installManifestHash
        Assert-PlainPath $installDestination
        # 只移动本次已验证的暂存目录，不覆盖已有安装或运行中的 EXE。
        [IO.Directory]::Move($installPayload, $installDestination)
    }
    Send-InstallEvent @{ kind = 'finished'; cli_path = (Join-Path $installDestination 'raishot-cli.exe'); version = $installVersion }
    exit 0
} catch {
    Send-InstallEvent @{ kind = 'failed'; message = ('RaiShot 插件安装失败：' + $_.Exception.Message) }
    exit 1
}
