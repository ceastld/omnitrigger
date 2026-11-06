# RaiShot与Legacy共用现有路径、SHA-256和有界HTTPS传输。
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
