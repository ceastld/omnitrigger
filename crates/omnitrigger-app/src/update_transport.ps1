. (Join-Path $PSScriptRoot 'common.ps1')
try {
    $updateStage = [IO.Path]::GetFullPath($env:OMNITRIGGER_UPDATE_STAGE)
    Assert-UpdatePath $updateStage
    if ($env:OMNITRIGGER_UPDATE_MODE -ceq 'check') {
        Write-UpdateProgress '正在检查更新…'
        $updateResponsePath = Join-Path $updateStage 'version.json'
        Get-UpdateFile $env:OMNITRIGGER_UPDATE_API $updateResponsePath 64KB
        $updateManifest = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($updateResponsePath))
        Write-UpdateEvent @{ kind = 'manifest'; manifest = $updateManifest }
    } elseif ($env:OMNITRIGGER_UPDATE_MODE -ceq 'fallback') {
        $updateResponsePath = Join-Path $updateStage 'releases.json'
        Get-UpdateFile 'https://api.github.com/repos/ceastld/omnitrigger/releases?per_page=100' $updateResponsePath 4MB
        $updateReleases = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($updateResponsePath))
        # 不把不需要的作者/下载统计等 API 字段传回界面线程。
        $updateResults = @($updateReleases | ForEach-Object {
            @{ tag_name = $_.tag_name; body = $_.body; draft = $_.draft; assets = @($_.assets | ForEach-Object {
                @{ name = $_.name; size = $_.size; browser_download_url = $_.browser_download_url; digest = $_.digest }
            }) }
        })
        Write-UpdateEvent @{ kind = 'releases'; releases = $updateResults }
    } elseif ($env:OMNITRIGGER_UPDATE_MODE -ceq 'install') {
        $updatePlan = [IO.File]::ReadAllText((Join-Path $updateStage 'plan.json')) | ConvertFrom-Json
        $updateArchiveName = "OmniTrigger-$($updatePlan.release.tag_name.Substring(1))-windows-x86_64.zip"
        $updateArchive = Join-Path $updateStage 'package.zip'
        foreach ($updateName in @('SHA256SUMS.txt', $updateArchiveName)) {
            $updateAssets = @($updatePlan.release.assets | Where-Object name -CEQ $updateName)
            if ($updateAssets.Count -ne 1 -or $updateAssets[0].browser_download_url -cne "https://github.com/ceastld/omnitrigger/releases/download/$($updatePlan.release.tag_name)/$updateName") { throw '发布资产无效。' }
            $updateAsset = $updateAssets[0]
            $updateDestination = if ($updateName -ceq 'SHA256SUMS.txt') { Join-Path $updateStage $updateName } else { $updateArchive }
            $updateLimit = if ($updateName -ceq 'SHA256SUMS.txt') { 64KB } else { 512MB }
            Get-UpdateFile $updateAsset.browser_download_url $updateDestination $updateLimit $updateAsset.size
        }
        Write-UpdateProgress '正在校验并准备更新…'
        Confirm-UpdateHash (Join-Path $updateStage 'SHA256SUMS.txt') $updateArchiveName $updateArchive
        Confirm-UpdateDigest $updateArchive $updatePlan.release.sha256
        $updateFiles = Expand-UpdateArchive $updateArchive (Join-Path $updateStage 'payload')
        [IO.File]::WriteAllText((Join-Path $updateStage 'files.json'), (ConvertTo-Json -InputObject @($updateFiles) -Compress), (New-Object Text.UTF8Encoding($false)))
        Write-UpdateEvent @{ kind = 'prepared' }
    } else { throw '未知更新操作。' }
} catch {
    Write-UpdateEvent @{ kind = 'failed'; message = $_.Exception.Message }
    exit 1
}
