# 浏览器连接器安装与生命周期。由应用后台运行；输入均通过环境传入。
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
$ProgressPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.Net.Http

function Send-Event([hashtable]$Value) { [Console]::WriteLine(($Value | ConvertTo-Json -Compress -Depth 30)) }
function Progress([string]$Message) { Send-Event @{kind='progress';message=$Message} }
function Fail([string]$Code, [string]$Message) { $script:errorCode=$Code; throw $Message }
function Assert-Plain([string]$Path) {
    $current=[IO.Path]::GetFullPath($Path)
    if ($current -match '(?i)(^|[\\/])(OneDrive[^\\/]*|Dropbox|iCloud[^\\/]*)([\\/]|$)') { Fail 'unsafe_path' '连接器必须使用普通本地目录。' }
    while ($current) {
        if (Test-Path -LiteralPath $current) {
            $item=Get-Item -LiteralPath $current -Force
            if (([int64]$item.Attributes -band (0x400 -bor 0x1000 -bor 0x40000 -bor 0x400000)) -ne 0) { Fail 'unsafe_path' '连接器路径包含链接、离线文件或云端占位。' }
        }
        $parent=[IO.Path]::GetDirectoryName($current)
        if ($parent -eq $current) { break }
        $current=$parent
    }
}
function Hash-File([string]$Path) {
    Assert-Plain $Path
    $stream=[IO.File]::OpenRead($Path)
    $algorithm=$null
    try {
        $algorithm=[Security.Cryptography.SHA256]::Create()
        return ([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-','').ToLowerInvariant()
    } finally {if ($null -ne $algorithm) {$algorithm.Dispose()};$stream.Dispose()}
}
function Atomic-Json([string]$Path, $Value) {
    Assert-Plain $Path
    $temporary=$Path+'.tmp-'+[Guid]::NewGuid().ToString('N')
    [IO.File]::WriteAllText($temporary,($Value | ConvertTo-Json -Depth 30), (New-Object Text.UTF8Encoding($false)))
    if ([IO.File]::Exists($Path)) {
        $backup=$Path+'.backup-'+[Guid]::NewGuid().ToString('N')
        [IO.File]::Replace($temporary,$Path,$backup)
        try { [IO.File]::Delete($backup) } catch {}
    } else { [IO.File]::Move($temporary,$Path) }
}
function Read-Json([string]$Path, [long]$Limit) {
    Assert-Plain $Path
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf) -or (Get-Item -LiteralPath $Path).Length -gt $Limit) { Fail 'invalid_manifest' '文件缺失或超过大小限制。' }
    return ([IO.File]::ReadAllText($Path) | ConvertFrom-Json)
}
function Trusted-Uri([uri]$Uri) {
    return ($Uri.Scheme -ceq 'https' -and $Uri.IsDefaultPort -and -not $Uri.UserInfo -and
        @('github.com','release-assets.githubusercontent.com','objects.githubusercontent.com') -contains $Uri.DnsSafeHost)
}
function Download([string]$Url,[string]$Destination,[long]$Limit,[long]$Expected) {
    $handler=New-Object Net.Http.HttpClientHandler
    $handler.AllowAutoRedirect=$false
    $client=New-Object Net.Http.HttpClient($handler)
    $client.Timeout=[TimeSpan]::FromSeconds(120)
    $client.DefaultRequestHeaders.UserAgent.ParseAdd('OmniTrigger-Browser-Connector/0.1')
    $response=$null
    try {
        $uri=[uri]$Url
        for ($redirect=0;$redirect -lt 6;$redirect++) {
            if (-not (Trusted-Uri $uri)) { Fail 'invalid_source' '下载地址不是配置的 HTTPS 发布源。' }
            $response=$client.GetAsync($uri,[Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
            if ([int]$response.StatusCode -in @(301,302,303,307,308)) {
                if ($null -eq $response.Headers.Location) { Fail 'download_failed' '发布源重定向无效。' }
                $uri=New-Object Uri($uri,$response.Headers.Location)
                $response.Dispose();$response=$null;continue
            }
            if ([int]$response.StatusCode -ne 200) { Fail 'release_unavailable' '浏览器连接器暂不可下载，请稍后重试。' }
            if ($response.Content.Headers.ContentLength -gt $Limit) { Fail 'payload_too_large' '下载内容超过大小限制。' }
            $input=$response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
            $output=[IO.File]::Open($Destination,[IO.FileMode]::CreateNew)
            try {
                $buffer=New-Object byte[] 65536;$total=0L;$deadline=[DateTime]::UtcNow.AddSeconds(120)
                while ($true) {
                    if ([DateTime]::UtcNow -gt $deadline) { Fail 'download_timeout' '下载超时。' }
                    $read=$input.ReadAsync($buffer,0,$buffer.Length)
                    if (-not $read.Wait(30000)) { Fail 'download_timeout' '发布源长时间没有响应。' }
                    $count=$read.GetAwaiter().GetResult()
                    if ($count -eq 0) { break }
                    $total+=$count
                    if ($total -gt $Limit) { Fail 'payload_too_large' '下载内容超过大小限制。' }
                    $output.Write($buffer,0,$count)
                }
                if ($Expected -ge 0 -and $total -ne $Expected) { Fail 'size_mismatch' '下载包大小与发布清单不符。' }
            } finally { $output.Dispose();$input.Dispose() }
            return
        }
        Fail 'download_failed' '发布源重定向次数过多。'
    } finally { if ($null -ne $response) {$response.Dispose()};$client.Dispose();$handler.Dispose() }
}
function Assert-Release($Manifest) {
    if ($Manifest.schemaVersion -ne 1 -or $Manifest.protocolVersion -ne 1 -or $Manifest.version -cne $script:version -or $Manifest.repository -cne 'ceastld/omnitrigger-browser') { Fail 'invalid_manifest' '发布清单的版本、协议或仓库不匹配。' }
    $identity=$Manifest.PSObject.Properties['extensionId']
    if ($null -eq $identity -or $identity.Value -isnot [string] -or $identity.Value -cne $script:fixedExtensionId) { Fail 'extension_identity_mismatch' '发布清单的 Chrome 扩展身份不匹配。' }
    $asset=$Manifest.artifacts.connector
    $name='OmniTrigger-Browser-Connector-'+$script:version+'-windows-x86_64.zip'
    $url='https://github.com/ceastld/omnitrigger-browser/releases/download/v'+$script:version+'/'+$name
    if ($asset.fileName -cne $name -or $asset.url -cne $url -or $asset.target -cne 'x86_64-pc-windows-msvc' -or
        $asset.sha256 -cnotmatch '^[a-f0-9]{64}$' -or ($asset.sizeBytes -isnot [int] -and $asset.sizeBytes -isnot [long]) -or $asset.sizeBytes -le 0 -or $asset.sizeBytes -gt 67108864) { Fail 'invalid_manifest' '连接器发布资产描述无效。' }
    return $asset
}
function Assert-Package([string]$Directory) {
    $meta=Read-Json (Join-Path $Directory 'connector.json') 65536
    if ($meta.schemaVersion -ne 1 -or $meta.protocolVersion -ne 1 -or $meta.version -cne $script:version -or $meta.hostName -cne 'com.omnitrigger.browser' -or
        $meta.entrypoints.cli -cne 'omnitrigger-browser.exe' -or $meta.entrypoints.nativeHost -cne 'omnitrigger-browser-host.exe') { Fail 'invalid_package' '连接器包的版本、协议或入口不匹配。' }
    $identity=$meta.PSObject.Properties['extensionId']
    if ($null -eq $identity -or $identity.Value -isnot [string] -or $identity.Value -cne $script:fixedExtensionId) { Fail 'extension_identity_mismatch' '连接器包的 Chrome 扩展身份不匹配。' }
    foreach ($file in @('omnitrigger-browser.exe','omnitrigger-browser-host.exe','LICENSE','THIRD_PARTY_NOTICES.md','third-party/index.json','sdk-js/index.mjs')) {
        $path=Join-Path $Directory $file;Assert-Plain $path
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { Fail 'invalid_package' ('连接器包缺少文件：'+$file) }
    }
}
function Install-Package([string]$ManifestPath,[string]$ArchivePath) {
    $manifest=Read-Json $ManifestPath 262144;$asset=Assert-Release $manifest
    Assert-Plain $ArchivePath
    if (-not (Test-Path -LiteralPath $ArchivePath -PathType Leaf) -or (Get-Item -LiteralPath $ArchivePath).Length -ne $asset.sizeBytes) { Fail 'size_mismatch' '连接器 ZIP 大小与发布清单不符。' }
    if ((Hash-File $ArchivePath) -cne $asset.sha256) { Fail 'checksum_mismatch' '连接器 ZIP 的 SHA-256 校验失败。' }
    $destination=Join-Path $script:root $script:version
    if (Test-Path -LiteralPath $destination) {
        $receipt=Read-Json (Join-Path $destination 'installation.json') 262144
        if ($receipt.sha256 -cne $asset.sha256) { Fail 'install_conflict' '此版本目录已包含另一发布包，未覆盖。' }
        Assert-Package $destination
        foreach ($file in $receipt.files) {
            if ($file.path -match '(^|[\\/])\.\.([\\/]|$)' -or [IO.Path]::IsPathRooted($file.path)) { Fail 'invalid_package' '安装收据路径无效。' }
            if ((Hash-File (Join-Path $destination $file.path)) -cne $file.sha256) { Fail 'checksum_mismatch' '已安装文件已改变，请先卸载或使用新的版本目录。' }
        }
        return $destination
    }
    $payload=Join-Path $script:stage 'payload';[void][IO.Directory]::CreateDirectory($payload)
    $archive=[IO.Compression.ZipFile]::OpenRead($ArchivePath)
    $files=@();$seen=New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    try {
        if ($archive.Entries.Count -gt 512) { Fail 'invalid_package' '连接器 ZIP 文件项过多。' }
        $total=0L
        foreach ($entry in $archive.Entries) {
            $name=$entry.FullName.Replace('\','/')
            if ([string]::IsNullOrEmpty($name) -or $name.StartsWith('/') -or $name.Contains(':') -or $name.Contains('//') -or
                $name -match '(^|/)\.{1,2}(/|$)' -or $name -match '[<>"|?*]' -or $name -match '(^|/)(CON|PRN|AUX|NUL|CONIN\$|CONOUT\$|COM[1-9]|LPT[1-9])([. ]|/|$)' -or
                $name -match '[. ](/|$)' -or -not $seen.Add($name.TrimEnd('/'))) { Fail 'unsafe_archive' 'ZIP 包含无效、重复或逃逸路径。' }
            $mode=([int64]$entry.ExternalAttributes -shr 16) -band 0xF000
            if ($mode -eq 0xA000 -or ($entry.ExternalAttributes -band 0x400) -ne 0) { Fail 'unsafe_archive' 'ZIP 不允许符号链接。' }
            if ($name -notmatch '^(omnitrigger-browser(?:-host)?\.exe|connector\.json|LICENSE|README\.md|THIRD_PARTY_NOTICES\.md|sdk-js/.*|installer/.*|third-party/.*)$') { Fail 'unsafe_archive' 'ZIP 包含未知顶层内容。' }
            $target=[IO.Path]::GetFullPath((Join-Path $payload $name))
            if (-not $target.StartsWith($payload+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { Fail 'unsafe_archive' 'ZIP 路径越界。' }
            $total+=$entry.Length
            if ($entry.Length -gt 33554432 -or $total -gt 67108864) { Fail 'payload_too_large' 'ZIP 解压大小超过限制。' }
            if ($name.EndsWith('/')) { [void][IO.Directory]::CreateDirectory($target);continue }
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
            $source=$entry.Open();$output=[IO.File]::Open($target,[IO.FileMode]::CreateNew)
            try {
                $buffer=New-Object byte[] 65536;$actual=0L
                while (($count=$source.Read($buffer,0,$buffer.Length)) -gt 0) {
                    $actual+=$count
                    if ($actual -gt $entry.Length) { Fail 'unsafe_archive' 'ZIP 实际解压大小超过声明值。' }
                    $output.Write($buffer,0,$count)
                }
                if ($actual -ne $entry.Length) { Fail 'unsafe_archive' 'ZIP 解压长度与声明值不符。' }
            } finally { $source.Dispose();$output.Dispose() }
            $files+=@{path=$name;sha256=(Hash-File $target)}
        }
    } finally { $archive.Dispose() }
    Assert-Package $payload
    Atomic-Json (Join-Path $payload 'installation.json') @{sha256=$asset.sha256;files=@($files)}
    Assert-Plain $destination
    [IO.Directory]::Move($payload,$destination)
    return $destination
}
function Invoke-Cli([string]$Arguments,[bool]$AllowFailure=$false) {
    $info=New-Object Diagnostics.ProcessStartInfo
    $info.FileName=$script:state.cli_path;$info.Arguments=$Arguments;$info.WorkingDirectory=[IO.Path]::GetDirectoryName($script:state.cli_path)
    $info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
    $info.EnvironmentVariables['OMNITRIGGER_BROWSER_CONFIG']=Join-Path $info.WorkingDirectory 'connection.json'
    $process=New-Object Diagnostics.Process;$process.StartInfo=$info
    try {
        [void]$process.Start();$out=$process.StandardOutput.ReadToEndAsync();$err=$process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(60000)) { $process.Kill();$process.WaitForExit();Fail 'connector_timeout' '连接器程序没有按时返回。' }
        $output=$out.GetAwaiter().GetResult();$errorText=$err.GetAwaiter().GetResult()
        if ($output.Length -gt 524288 -or $errorText.Length -gt 16384) { Fail 'connector_output' '连接器程序回执过大。' }
        if ($process.ExitCode -ne 0) {
            if ($AllowFailure) { return $null }
            $message='连接器程序执行失败。';$code='connector_failed'
            try {
                $json=$errorText | ConvertFrom-Json
                if ($json.error.message) { $message=[string]$json.error.message }
                if ($json.error.code -match '^[a-z_]{1,64}$') { $code=[string]$json.error.code }
            } catch {}
            Fail $code $message
        }
        return $output.Trim()
    } finally { $process.Dispose() }
}
function Owned-Bridge {
    if (-not $script:state.bridge_pid) { return $null }
    $process=Get-Process -Id $script:state.bridge_pid -ErrorAction SilentlyContinue
    if ($null -eq $process) { return $null }
    try {
        if (-not [string]::Equals($process.Path,$script:state.cli_path,[StringComparison]::OrdinalIgnoreCase) -or $process.StartTime.ToUniversalTime().ToString('o') -cne $script:state.bridge_started_at) { return $null }
        return $process
    } catch { return $null }
}
function Stop-Owned {
    $process=Owned-Bridge
    if ($null -ne $process) { Stop-Process -Id $process.Id -ErrorAction Stop }
    $script:state.bridge_pid=$null;$script:state.bridge_started_at='';$script:state.enabled=$false
}
function Registration {
    $directory=[IO.Path]::GetDirectoryName($script:state.cli_path)
    if (-not (Test-Path -LiteralPath (Join-Path $directory 'connection.json') -PathType Leaf)) {
        return [pscustomobject]@{registered=$false;conflict=$null;pipe=$null}
    }
    return ((Invoke-Cli ('registration-status --install-dir "'+$directory+'"')) | ConvertFrom-Json)
}
function Same-Path([string]$Left,[string]$Right) {
    if (-not $Left -or -not $Right) {return $false}
    try {return [string]::Equals([IO.Path]::GetFullPath($Left),[IO.Path]::GetFullPath($Right),[StringComparison]::OrdinalIgnoreCase)} catch {return $false}
}
function Registered-Manifest {
    if (Test-Path -LiteralPath $script:registryPath) {return [string](Get-Item -LiteralPath $script:registryPath).GetValue('')}
    return $null
}
function Test-LegacyInstallation([string]$ManifestPath) {
    try {
        $legacyRoot=[IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'OmniTriggerBrowser'))
        $expectedManifest=Join-Path $legacyRoot 'com.omnitrigger.browser.json'
        $expectedHost=Join-Path $legacyRoot 'omnitrigger-browser-host.exe'
        if (-not (Same-Path $ManifestPath $expectedManifest)) {return $false}
        Assert-Plain $legacyRoot;Assert-Plain $expectedHost
        if (-not (Test-Path -LiteralPath $expectedHost -PathType Leaf)) {return $false}
        $manifest=Read-Json $expectedManifest 65536
        $connection=Read-Json (Join-Path $legacyRoot 'connection.json') 65536
        if ($manifest.name -isnot [string] -or $manifest.type -isnot [string] -or $manifest.path -isnot [string] -or $manifest.name -cne 'com.omnitrigger.browser' -or $manifest.type -cne 'stdio' -or -not (Same-Path $manifest.path $expectedHost)) {return $false}
        $defaultPipe=Invoke-Cli 'endpoint'
        if ($connection.pipe -isnot [string] -or $connection.pipe -cne $defaultPipe -or $connection.extension_ids -isnot [Array] -or $manifest.allowed_origins -isnot [Array]) {return $false}
        $ids=New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)
        foreach ($id in $connection.extension_ids) {if ($id -isnot [string] -or $id -cnotmatch '^[a-p]{32}$' -or -not $ids.Add($id)) {return $false}}
        if ($ids.Count -eq 0 -or $ids.Count -gt 32 -or $manifest.allowed_origins.Count -ne $ids.Count) {return $false}
        foreach ($origin in $manifest.allowed_origins) {
            if ($origin -isnot [string] -or $origin -cnotmatch '^chrome-extension://[a-p]{32}/$' -or -not $ids.Remove($origin.Substring(19,32))) {return $false}
        }
        return $ids.Count -eq 0
    } catch {return $false}
}
function Register-Connector([string]$Id,[bool]$MigrateLegacy=$false) {
    if ($Id -cnotmatch '^[a-p]{32}$') {Fail 'invalid_extension_id' 'Chrome 扩展身份无效。'}
    $directory=[IO.Path]::GetDirectoryName($script:state.cli_path)
    $expectedManifest=Join-Path $directory 'com.omnitrigger.browser.json'
    $previous=Registered-Manifest
    $script:registrationBefore=$previous;$script:newManifest=$expectedManifest
    if ($previous -and -not (Same-Path $previous $expectedManifest)) {
        if (-not $MigrateLegacy -or -not (Test-LegacyInstallation $previous)) {Fail 'registration_conflict' 'Chrome 本机连接已属于另一安装，未覆盖原有注册。'}
        $backup=$script:statePath+'.previous-registration-'+[Guid]::NewGuid().ToString('N')+'.json'
        Atomic-Json $backup @{schemaVersion=1;manifestPath=$previous;createdAt=[DateTime]::UtcNow.ToString('o')}
        if ((Registered-Manifest) -cne $previous) {Fail 'registration_conflict' '本机连接注册在迁移前发生变化，未覆盖。'}
        $script:registrationChanged=$true
        Remove-Item -LiteralPath $script:registryPath
        Progress '正在迁移已有的 OmniTrigger 浏览器连接。'
    } elseif (-not $previous) {$script:registrationChanged=$true}
    $hash=[Security.Cryptography.SHA256]::Create()
    try {$suffix=([BitConverter]::ToString($hash.ComputeHash([Text.Encoding]::UTF8.GetBytes($script:root.ToLowerInvariant())))).Replace('-','').Substring(0,24).ToLowerInvariant()} finally {$hash.Dispose()}
    $pipe='\\.\pipe\omnitrigger-browser-plugin-'+$suffix
    $registered=(Invoke-Cli ('register --extension-id '+$Id+' --install-dir "'+$directory+'" --pipe "'+$pipe+'"')) | ConvertFrom-Json
    if ($registered.conflict -or -not $registered.registered) {Fail 'registration_failed' '连接器没有确认本机注册成功。'}
    $script:state.extension_id=$Id;$script:state.registered=$true
}
function Start-Connector {
    $directory=[IO.Path]::GetDirectoryName($script:state.cli_path)
    $registration=Registration
    if (-not $registration.registered -or $registration.conflict) {Fail 'registration_required' '浏览器本机连接尚未注册成功。'}
    $script:state.registered=$true
    $existing=Invoke-Cli 'profiles' $true
    if ($null -eq $existing) {
        # 无重定向的隐藏独立进程不会继承安装器回执管道。
        $process=Start-Process -FilePath $script:state.cli_path -ArgumentList @('serve','--pipe',('"'+$registration.pipe+'"')) -WorkingDirectory $directory -WindowStyle Hidden -PassThru
        $script:state.bridge_pid=$process.Id;$script:state.bridge_started_at=$process.StartTime.ToUniversalTime().ToString('o');$script:launchedHere=$true
        $ready=$false
        for ($try=0;$try -lt 20;$try++) {Start-Sleep -Milliseconds 150;$existing=Invoke-Cli 'profiles' $true;if ($null -ne $existing) {$ready=$true;break};if ($process.HasExited) {break}}
        if (-not $ready) {Stop-Owned;Fail 'bridge_start_failed' '浏览器连接器未能就绪，请重试安装。'}
    }
    $script:profiles=@();$decodedProfiles=ConvertFrom-Json -InputObject $existing
    foreach ($profile in $decodedProfiles) {$script:profiles+=,$profile}
    $script:state.enabled=$true
}
function Restore-PreviousRegistration {
    if (-not $script:registrationChanged) {return}
    $current=Registered-Manifest
    if ($current -and -not (Same-Path $current $script:newManifest)) {return}
    if ($current) {
        if (-not (Same-Path (Registered-Manifest) $script:newManifest)) {return}
        Remove-Item -LiteralPath $script:registryPath
    }
    if ($script:registrationBefore -and -not (Registered-Manifest)) {
        [void](New-Item -Path $script:registryPath -Force)
        Set-Item -LiteralPath $script:registryPath -Value $script:registrationBefore
    }
}
function Is-OurSharedConfig($Config,[string]$Pipe) {
    if ($Config.pipe -isnot [string] -or $Config.pipe -cne $Pipe -or $Config.extension_ids -isnot [Array] -or $Config.extension_ids.Count -ne 1) {return $false}
    foreach ($property in $Config.PSObject.Properties) {if ($property.Name -notin @('pipe','extension_ids')) {return $false}}
    return $Config.extension_ids[0] -is [string] -and $Config.extension_ids[0] -ceq $script:fixedExtensionId
}
function Publish-SharedClientConfig {
    $directory=[IO.Path]::GetDirectoryName($script:state.cli_path)
    $expectedManifest=Join-Path $directory 'com.omnitrigger.browser.json'
    if (-not (Same-Path (Registered-Manifest) $expectedManifest)) {Fail 'registration_conflict' '浏览器连接归属发生变化，未发布客户端配置。'}
    $connection=Read-Json (Join-Path $directory 'connection.json') 65536
    if (-not (Is-OurSharedConfig $connection $connection.pipe)) {Fail 'invalid_config' '连接器自身客户端配置无效。'}
    $sharedRoot=[IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'OmniTriggerBrowser'))
    $script:sharedConfigPath=Join-Path $sharedRoot 'connection.json'
    Assert-Plain $sharedRoot;Assert-Plain $script:sharedConfigPath
    if (Test-Path -LiteralPath $script:sharedConfigPath) {
        $existing=Read-Json $script:sharedConfigPath 65536
        $recognized=$false
        try {$recognized=Is-OurSharedConfig $existing $connection.pipe} catch {}
        if (-not $recognized) {$recognized=Test-LegacyInstallation (Join-Path $sharedRoot 'com.omnitrigger.browser.json')}
        if (-not $recognized) {Fail 'client_config_conflict' '现有浏览器客户端配置不属于已识别安装，未覆盖。'}
        $backupRoot=Join-Path $script:root '.client-backups';Assert-Plain $backupRoot
        [void][IO.Directory]::CreateDirectory($backupRoot)
        $script:sharedBackupPath=Join-Path $backupRoot ('connection-'+[Guid]::NewGuid().ToString('N')+'.json')
        [IO.File]::Copy($script:sharedConfigPath,$script:sharedBackupPath)
    }
    [void][IO.Directory]::CreateDirectory($sharedRoot)
    if (-not (Same-Path (Registered-Manifest) $expectedManifest)) {Fail 'registration_conflict' '浏览器连接归属发生变化，未发布客户端配置。'}
    Atomic-Json $script:sharedConfigPath @{pipe=[string]$connection.pipe;extension_ids=@($script:fixedExtensionId)}
    $script:sharedPublishedHash=Hash-File $script:sharedConfigPath
    $script:sharedPublished=$true
}
function Restore-SharedClientConfig {
    if (-not $script:sharedPublished) {return}
    $expectedManifest=Join-Path ([IO.Path]::GetDirectoryName($script:state.cli_path)) 'com.omnitrigger.browser.json'
    if (-not (Same-Path (Registered-Manifest) $expectedManifest) -or (Hash-File $script:sharedConfigPath) -cne $script:sharedPublishedHash) {return}
    if ($script:sharedBackupPath) {
        $previous=Read-Json $script:sharedBackupPath 65536
        Atomic-Json $script:sharedConfigPath $previous
    } else {[IO.File]::Delete($script:sharedConfigPath)}
}

$script:errorCode='connector_failed'
$script:launchedHere=$false
$script:registrationChanged=$false
$script:registrationBefore=$null
$script:newManifest=$null
$script:sharedPublished=$false
$script:sharedPublishedHash=$null
$script:sharedConfigPath=$null
$script:sharedBackupPath=$null
$script:registryPath='HKCU:\Software\Google\Chrome\NativeMessagingHosts\com.omnitrigger.browser'
$script:fixedExtensionId='amgcemaohhmiopijabpaooapbkfokkjb'
try {
    $script:root=[IO.Path]::GetFullPath($env:OMNITRIGGER_BROWSER_ROOT)
    $script:stage=[IO.Path]::GetFullPath($env:OMNITRIGGER_BROWSER_STAGE)
    $script:statePath=[IO.Path]::GetFullPath($env:OMNITRIGGER_BROWSER_STATE)
    $script:version=$env:OMNITRIGGER_BROWSER_VERSION
    if ($script:version -cne '0.1.0' -or [IO.Path]::GetDirectoryName($script:stage) -cne $script:root -or -not [IO.Path]::GetFileName($script:stage).StartsWith('.install-')) { Fail 'invalid_request' '连接器操作目录或版本无效。' }
    Assert-Plain $script:root;Assert-Plain $script:stage;Assert-Plain $statePath
    $script:state=[ordered]@{enabled=$false;extension_id='';version='';cli_path='';installed=$false;registered=$false;bridge_pid=$null;bridge_started_at=''}
    if (Test-Path -LiteralPath $statePath) {
        $saved=Read-Json $statePath 65536
        foreach ($property in $saved.PSObject.Properties) {
            if (-not $script:state.Contains($property.Name)) { Fail 'settings_invalid' '连接器设置包含未知字段，原文件已保留。' }
            $script:state[$property.Name]=$property.Value
        }
    }
    $expected=Join-Path (Join-Path $script:root $script:version) 'omnitrigger-browser.exe'
    if ($script:state.cli_path -and $script:state.cli_path -cne $expected) { Fail 'settings_invalid' '连接器设置指向安装目录以外。' }
    if (($script:state.version -and $script:state.version -cne $script:version) -or ($script:state.extension_id -and $script:state.extension_id -cnotmatch '^[a-p]{32}$')) { Fail 'settings_invalid' '连接器设置的版本或扩展 ID 无效，原文件已保留。' }
    $profiles=@();$message='连接器状态已刷新。';$mode=$env:OMNITRIGGER_BROWSER_OPERATION
    if ($mode -eq 'install') {
        Progress '正在校验浏览器连接器发布包。'
        $manifestPath=Join-Path $script:stage 'release-manifest.json';$archivePath=Join-Path $script:stage 'connector.zip'
        $url=$env:OMNITRIGGER_BROWSER_RELEASE_MANIFEST
        if ($url -cne ('https://github.com/ceastld/omnitrigger-browser/releases/download/v'+$script:version+'/release-manifest.json')) { Fail 'invalid_source' '连接器发布清单地址不匹配。' }
        Download $url $manifestPath 262144 -1
        $release=Read-Json $manifestPath 262144
        $asset=Assert-Release $release
        Progress '正在下载浏览器连接器。'
        Download $asset.url $archivePath ([long]$asset.sizeBytes) ([long]$asset.sizeBytes)
        $directory=Install-Package $manifestPath $archivePath
        $script:state.version=$script:version;$script:state.cli_path=Join-Path $directory 'omnitrigger-browser.exe';$script:state.installed=$true
        $package=Read-Json (Join-Path $directory 'connector.json') 65536
        if ($package.extensionId -cne $release.extensionId) {Fail 'extension_identity_mismatch' '连接器与发布清单的 Chrome 扩展身份不一致。'}
        Progress '正在配置并启用浏览器连接。'
        Register-Connector $release.extensionId $true
        Start-Connector
        Publish-SharedClientConfig
        $message='浏览器连接器已安装并启用。'
    } elseif ($mode -in @('register','start','stop','refresh','uninstall')) {
        $script:state.installed=($script:state.cli_path -and (Test-Path -LiteralPath $script:state.cli_path -PathType Leaf))
        if ($mode -ne 'refresh' -and -not $script:state.installed) { Fail 'not_installed' '请先安装浏览器连接器。' }
        if ($script:state.installed) {
            $directory=[IO.Path]::GetDirectoryName($script:state.cli_path);Assert-Plain $directory
            if ($mode -eq 'register') {
                $id=$env:OMNITRIGGER_BROWSER_EXTENSION_ID
                Register-Connector $id
                $message='Chrome Native Host 已注册。'
            } elseif ($mode -eq 'start') {
                Start-Connector
                $message='本机连接器已就绪。'
            } elseif ($mode -eq 'stop') {
                Stop-Owned;$message='已停止本插件启动的连接器服务。'
            } elseif ($mode -eq 'uninstall') {
                $registration=Registration
                if ($registration.registered) { [void](Invoke-Cli ('unregister --install-dir "'+$directory+'"')) }
                Stop-Owned
                if ([IO.Path]::GetDirectoryName($directory) -cne $script:root -or [IO.Path]::GetFileName($directory) -cne $script:version) { Fail 'unsafe_path' '未删除不属于当前版本的目录。' }
                Assert-Plain $directory
                Remove-Item -LiteralPath $directory -Recurse -Force
                $script:state.enabled=$false;$script:state.installed=$false;$script:state.registered=$false;$script:state.cli_path='';$script:state.version='';$message='浏览器连接器已卸载，Chrome 资料未改变。'
            } else {
                $registration=Registration;$script:state.registered=[bool]$registration.registered
                $response=$null
                if (Test-Path -LiteralPath (Join-Path $directory 'connection.json') -PathType Leaf) { $response=Invoke-Cli 'profiles' $true }
                if ($null -ne $response) {
                    $decodedProfiles=ConvertFrom-Json -InputObject $response
                    foreach ($profile in $decodedProfiles) { $profiles+=,$profile }
                    $script:state.enabled=$true
                } else {$script:state.enabled=$false}
                if ($null -eq (Owned-Bridge)) {$script:state.bridge_pid=$null;$script:state.bridge_started_at=''}
                if ($registration.conflict) {$message='连接器已安装，但 Native Host 注册属于另一安装。'}
            }
        }
    } else { Fail 'invalid_request' '不支持的连接器操作。' }
    Atomic-Json $statePath $script:state
    Send-Event @{kind='finished';snapshot=$script:state;profiles=@($profiles);message=$message}
} catch {
    $failureCode=$script:errorCode;$failureMessage=$_.Exception.Message
    $recoveryFailed=$false
    if ($script:launchedHere) {try {Stop-Owned} catch {$recoveryFailed=$true}}
    try {Restore-SharedClientConfig} catch {$recoveryFailed=$true}
    try {Restore-PreviousRegistration} catch {$recoveryFailed=$true}
    if ($recoveryFailed) {$failureCode='rollback_failed';$failureMessage+=' 旧连接状态未能完全恢复，请重试安装。'}
    Send-Event @{kind='failed';code=$failureCode;message=$failureMessage}
    exit 1
}
