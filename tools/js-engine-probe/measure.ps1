#requires -Version 7.0
<#
.SYNOPSIS
构建两种独立 JS 宿主，测量 EXE、仅含 EXE 的 ZIP 和完整进程往返耗时。
#>
[CmdletBinding()]
param([ValidateRange(3, 100)][int]$Samples = 11)

$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw '此脚本测量 Windows x64 MSVC 产物。' }
if ((& rustc -vV) -notcontains 'host: x86_64-pc-windows-msvc') { throw '需要 Windows x64 MSVC Rust 工具链。' }
$outputRoot = Join-Path $PSScriptRoot 'results'
$null = New-Item -ItemType Directory -Path $outputRoot -Force
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere)) { throw '需要 vswhere 定位 MSVC dumpbin。' }
$dumpbin = @(& $vswhere -latest -products '*' -find 'VC/Tools/MSVC/*/bin/Hostx64/x64/dumpbin.exe') | Sort-Object | Select-Object -Last 1
if (-not $dumpbin) { throw '未找到 MSVC dumpbin。' }

function Invoke-Probe {
    param([string]$Executable)
    $process = [Diagnostics.Process]::new()
    try {
        $process.StartInfo.FileName = $Executable
        $process.StartInfo.UseShellExecute = $false
        $process.StartInfo.CreateNoWindow = $true
        $process.StartInfo.RedirectStandardInput = $true
        $process.StartInfo.RedirectStandardOutput = $true
        $process.StartInfo.RedirectStandardError = $true
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $null = $process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.StandardInput.Write('{"script":"return { sum: ctx.input.values.reduce((a,b) => a+b, 0) };","input":{"values":[1,2,3]}}')
        $process.StandardInput.Close()
        if (-not $process.WaitForExit(10000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw '宿主调用超过 10 秒。'
        }
        $clock.Stop()
        if ($process.ExitCode -ne 0) { throw "宿主失败：$($stdout.Result) $($stderr.Result)" }
        $response = $stdout.Result | ConvertFrom-Json
        if ($response.ok -ne $true -or $response.result.sum -ne 6) { throw "回执不符：$($stdout.Result)" }
        [Math]::Round($clock.Elapsed.TotalMilliseconds, 3)
    } finally {
        $process.Dispose()
    }
}

Push-Location -LiteralPath $PSScriptRoot
try {
    $measurements = foreach ($engine in @('boa', 'quickjs')) {
        foreach ($profile in @('release', 'small')) {
            $buildArguments = @('build', '--locked', '--no-default-features', '--features', $engine, '--profile', $profile, '--message-format=json-render-diagnostics')
            if ($engine -eq 'quickjs') { $buildArguments += @('--target-dir', 'target/quickjs') }
            # 显式 target 避免将静态 CRT 标志应用到宿主 proc-macro；cc 同步使用 /MT。
            if ($profile -eq 'small') {
                $buildArguments += @('--target', 'x86_64-pc-windows-msvc', '--config', 'target.x86_64-pc-windows-msvc.rustflags=["-C","target-feature=+crt-static"]')
            }
            $messages = & cargo @buildArguments |
                ForEach-Object { $_ | ConvertFrom-Json }
            if ($LASTEXITCODE -ne 0) { throw "$engine / $profile 构建失败。" }
            $artifacts = @($messages | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'omnitrigger-js-probe' -and $_.executable })
            if ($artifacts.Count -ne 1) { throw 'Cargo 未返回唯一 EXE。' }
            $executable = Join-Path $outputRoot "$engine-$profile.exe"
            Copy-Item -LiteralPath $artifacts[0].executable -Destination $executable -Force
            $imports = @(& $dumpbin /nologo /dependents $executable | ForEach-Object {
                if ($_ -match '^\s*(\S+\.dll)\s*$') { $Matches[1] }
            })
            if ($LASTEXITCODE -ne 0 -or $imports.Count -eq 0) { throw 'PE 依赖检查失败。' }
            if ($profile -eq 'small' -and ($imports -match '^(VCRUNTIME|MSVCP|MSVCR)')) { throw '静态 CRT 版本仍依赖额外 VC DLL。' }
            $zip = Join-Path $outputRoot "$engine-$profile.exe-only.zip"
            Compress-Archive -LiteralPath $executable -DestinationPath $zip -CompressionLevel Optimal -Force
            $first = Invoke-Probe -Executable $executable
            $timings = @(1..$Samples | ForEach-Object { Invoke-Probe -Executable $executable } | Sort-Object)
            [ordered]@{
                engine = $engine
                profile = $profile
                crt = $(if ($profile -eq 'small') { 'static' } else { 'dynamic' })
                imported_dlls = $imports
                exe_bytes = (Get-Item -LiteralPath $executable).Length
                exe_only_zip_bytes = (Get-Item -LiteralPath $zip).Length
                sha256 = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
                first_round_trip_ms = $first
                median_round_trip_ms = $timings[[int][Math]::Floor($Samples / 2)]
                round_trips_ms = $timings
            }
        }
    }
    $report = [ordered]@{
        api_version = 1
        measured_at_utc = [DateTime]::UtcNow.ToString('o')
        target = 'x86_64-pc-windows-msvc'
        rustc = @(& rustc -Vv)
        lock_sha256 = (Get-FileHash -LiteralPath Cargo.lock -Algorithm SHA256).Hash.ToLowerInvariant()
        samples = $Samples
        measurements = @($measurements)
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot 'measurements.json') -Encoding utf8
    $measurements | ForEach-Object { [PSCustomObject]$_ } | Format-Table engine, profile, exe_bytes, exe_only_zip_bytes, median_round_trip_ms
} finally {
    Pop-Location
}
