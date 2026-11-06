#requires -Version 7.2
param(
    [ValidateSet('Node','QuickJS','Boa')][string]$Runtime,
    [string]$ExePath,
    [string]$RuntimePath,
    [string]$CaseRoot,
    [switch]$EntrySmoke,
    [switch]$RunNative
)
$ErrorActionPreference='Stop'
if(-not $EntrySmoke -and -not $RunNative){
    @{status='not_run';entry_cli_executed=$false;selection_UIA_executed=$false;required='显式选择Runtime、ExePath、RuntimePath；EntrySmoke仅验入口，RunNative才创建自有Edit窗口'} | ConvertTo-Json -Compress
    return
}
if(-not $IsWindows){throw '真实CLI及Native验收需要Windows。'}
if(-not $Runtime -or -not $ExePath -or -not $RuntimePath){throw '必须显式提供Runtime、ExePath和RuntimePath。'}
if($RunNative -and [Threading.Thread]::CurrentThread.ApartmentState -ne 'STA'){throw 'RunNative请使用pwsh -STA -File。'}
function Assert-LocalFile([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if ($full.StartsWith('\\') -or $full -match '(?i)[\\/](OneDrive[^\\/]*|Dropbox|iCloud[^\\/]*)[\\/]') { throw '只接受普通本地文件。' }
    $item = Get-Item -LiteralPath $full -Force
    $part = $item
    while ($null -ne $part) {
        if (([int64]$part.Attributes -band 0x441400) -ne 0) { throw '拒绝联接、云端占位或离线文件。' }
        if ($part -is [IO.FileInfo]) { $part = $part.Directory } else { $part = $part.Parent }
    }
    if (-not (Test-Path -LiteralPath $full -PathType Leaf)) { throw '文件不存在。' }
    return $full
}

$ExePath=Assert-LocalFile $ExePath
$RuntimePath=Assert-LocalFile $RuntimePath
if(-not $CaseRoot){$CaseRoot=Join-Path ([IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))) ('.tmp/selection-read-'+[Guid]::NewGuid().ToString('N'))}
$testRoot=[IO.Path]::GetFullPath($CaseRoot)
if($testRoot.StartsWith('\\') -or $testRoot -match '(?i)[\\/](OneDrive[^\\/]*|Dropbox|iCloud[^\\/]*)[\\/]'){throw '测试目录必须为普通本地目录。'}
if(Test-Path -LiteralPath $testRoot){throw 'CaseRoot必须是全新目录，不能覆盖既有文件。'}
$parent=Get-Item -LiteralPath ([IO.Path]::GetDirectoryName($testRoot)) -Force
while($null -ne $parent){if(([int64]$parent.Attributes -band 0x441400) -ne 0){throw '测试目录祖先包含云端占位或联接。'};$parent=$parent.Parent}
$package=Join-Path $testRoot 'actions/selection-read'
[IO.Directory]::CreateDirectory($package)|Out-Null
$utf8=[Text.UTF8Encoding]::new($false,$true)
function Save-Json([string]$Path,$Value){[IO.File]::WriteAllText($Path,($Value|ConvertTo-Json -Depth 15 -Compress),$utf8)}
Save-Json (Join-Path $testRoot 'bindings.json') @{version=1;bindings=@()}
$kind=if($Runtime -eq 'Node'){'node'}else{'script'}
Save-Json (Join-Path $package 'action.json') @{schema_version=1;id='test.selection-read';name='自建Edit选区验收';version='1.0.0';actions=@(@{id='read';title='读取原目标';version=1;runtime=$kind;entry='main.mjs'})}
$source=Assert-LocalFile (Join-Path $PSScriptRoot 'selection-read-action.mjs')
[IO.File]::WriteAllText((Join-Path $package 'main.mjs'),[IO.File]::ReadAllText($source,$utf8),$utf8)
$previousNode=[Environment]::GetEnvironmentVariable('OMNITRIGGER_NODE','Process')
$previousScript=[Environment]::GetEnvironmentVariable('OMNITRIGGER_SCRIPT_HOST','Process')
$form=$null;$other=$null;$selectionUiReady=$false
$selectionCliSequence=0;$selectionCaseSequence=0;$selectionExpectedForeground=[IntPtr]::Zero
$selectionForegroundMismatch=$null;$selectionRunFailure=$null
$selectionCliCleanups=[Collections.Generic.List[object]]::new()
function Invoke-OwnedCli([string[]]$Arguments,[string]$Program=$ExePath){
    $script:selectionCliSequence+=1
    $receiptPath=Join-Path $testRoot ('cli-{0:d3}.json' -f $script:selectionCliSequence)
    $startedAt=[DateTimeOffset]::UtcNow.ToString('O');$timer=[Diagnostics.Stopwatch]::StartNew()
    $start=[Diagnostics.ProcessStartInfo]::new($Program);$start.UseShellExecute=$false;$start.CreateNoWindow=$true
    $start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    foreach($argument in $Arguments){$start.ArgumentList.Add($argument)}
    $child=[Diagnostics.Process]::new();$child.StartInfo=$start;$started=$false
    $out=[IO.MemoryStream]::new();$err=[IO.MemoryStream]::new()
    $outTask=$null;$errTask=$null;$outDone=$false;$errDone=$false;$captureTruncated=$false
    $cliPid=$null;$failure=$null;$naturalExit=$false;$exitBeforeCleanup=$null
    $killRequested=$false;$cleanupErrors=[Collections.Generic.List[string]]::new();$cleanupConfirmed=$false
    try{
        if(-not $child.Start()){throw '测试CLI未启动。'};$started=$true;$cliPid=$child.Id
        $outBytes=[byte[]]::new(4096);$errBytes=[byte[]]::new(4096)
        $outTask=$child.StandardOutput.BaseStream.ReadAsync($outBytes,0,$outBytes.Length)
        $errTask=$child.StandardError.BaseStream.ReadAsync($errBytes,0,$errBytes.Length)
        $drain={
            if(-not $outDone -and $outTask.IsCompleted){$count=$outTask.GetAwaiter().GetResult();if($count -eq 0){$outDone=$true}else{$take=[Math]::Min($count,[int][Math]::Max(0,65536-$out.Length-$err.Length));$out.Write($outBytes,0,$take);if($take -ne $count){$captureTruncated=$true};$outTask=$child.StandardOutput.BaseStream.ReadAsync($outBytes,0,$outBytes.Length)}}
            if(-not $errDone -and $errTask.IsCompleted){$count=$errTask.GetAwaiter().GetResult();if($count -eq 0){$errDone=$true}else{$take=[Math]::Min($count,[int][Math]::Max(0,65536-$out.Length-$err.Length));$err.Write($errBytes,0,$take);if($take -ne $count){$captureTruncated=$true};$errTask=$child.StandardError.BaseStream.ReadAsync($errBytes,0,$errBytes.Length)}}
        }
        while(-not ($child.HasExited -and $outDone -and $errDone)){
            if($selectionUiReady){
                [Windows.Forms.Application]::DoEvents()
                $foreground=[SelectionEditNative]::GetForegroundWindow()
                if($selectionExpectedForeground -ne [IntPtr]::Zero -and $foreground -ne $selectionExpectedForeground){
                    [uint32]$foregroundPid=0;$null=[SelectionEditNative]::GetWindowThreadProcessId($foreground,[ref]$foregroundPid)
                    $script:selectionForegroundMismatch=@{expected=$selectionExpectedForeground.ToInt64().ToString();actual=$foreground.ToInt64().ToString();actual_pid=$foregroundPid;elapsed_ms=$timer.ElapsedMilliseconds}
                    throw '前台偏离预期窗口；立即停止本次验收，不恢复前台或重试。'
                }
            }
            . $drain
            if($captureTruncated){throw '测试CLI输出超过合计64KiB，保留有界原始前缀。'}
            if($timer.ElapsedMilliseconds -gt 15000){throw '测试CLI/管道超时，不能确认成功。'}
            Start-Sleep -Milliseconds 2
        }
        $naturalExit=$true;$exitBeforeCleanup=$child.ExitCode
    }catch{$failure=$_.Exception.Message
    }finally{
        if($started){
            try{
                if($child.HasExited){if($null -eq $exitBeforeCleanup){$exitBeforeCleanup=$child.ExitCode;$naturalExit=$true}}
                else{$killRequested=$true;$child.Kill($true);if(-not $child.WaitForExit(3000)){throw '已知CLI进程树退出未确认。'}}
            }catch{$cleanupErrors.Add($_.Exception.Message)}
            $drainTimer=[Diagnostics.Stopwatch]::StartNew()
            while((-not $outDone -or -not $errDone) -and $drainTimer.ElapsedMilliseconds -lt 3000){try{. $drain}catch{$cleanupErrors.Add($_.Exception.Message);break};Start-Sleep -Milliseconds 2}
            try{$cleanupConfirmed=$child.HasExited -and $outDone -and $errDone -and $cleanupErrors.Count -eq 0}catch{$cleanupErrors.Add($_.Exception.Message)}
        }else{$cleanupConfirmed=$true}
        $actualExit=$null;if($started -and $child.HasExited){$actualExit=$child.ExitCode}
        $stdout=$null;$stderr=$null
        try{$stdout=$utf8.GetString($out.ToArray())}catch{$cleanupErrors.Add('stdout不是有效UTF8，保留base64')}
        try{$stderr=$utf8.GetString($err.ToArray())}catch{$cleanupErrors.Add('stderr不是有效UTF8，保留base64')}
        if($cleanupErrors.Count -ne 0){$cleanupConfirmed=$false}
        Save-Json $receiptPath @{program=$Program;arguments=$Arguments;cli_pid=$cliPid;caller_pid=$PID;started_at=$startedAt;elapsed_ms=$timer.ElapsedMilliseconds;natural_exit=$naturalExit;exit_before_cleanup=$exitBeforeCleanup;actual_exit_code=$actualExit;stdout=$stdout;stderr=$stderr;stdout_base64=[Convert]::ToBase64String($out.ToArray());stderr_base64=[Convert]::ToBase64String($err.ToArray());capture_truncated=$captureTruncated;failure=$failure;foreground_mismatch=$selectionForegroundMismatch;cleanup=@{kill_requested=$killRequested;confirmed=$cleanupConfirmed;stdout_eof=$outDone;stderr_eof=$errDone;errors=$cleanupErrors.ToArray()}}
        $script:selectionCliCleanups.Add(@{pid=$cliPid;actual_exit_code=$actualExit;confirmed=$cleanupConfirmed;kill_requested=$killRequested;receipt=$receiptPath})
        $child.Dispose();$out.Dispose();$err.Dispose()
    }
    if($failure){throw ($failure+'；原始CLI回执：'+$receiptPath)}
    if(-not $cleanupConfirmed -or $cleanupErrors.Count -ne 0){throw ('CLI/管道清理或输出解码未确认；原始回执：'+$receiptPath)}
    return @{code=$actualExit;stdout=$stdout;stderr=$stderr;pid=$cliPid;receipt=$receiptPath}
}
function Target-Context {
    [uint32]$owner=0
    $thread=[SelectionEditNative]::GetWindowThreadProcessId($form.Handle,[ref]$owner)
    if($owner -ne $PID -or $thread -eq 0){throw '自建目标身份不匹配。'}
    return @{window_handle=$form.Handle.ToInt64().ToString();process_id=$PID;thread_id=$thread
        process_created_file_time=([Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime().ToFileTimeUtc()).ToString()
        expires_at_unix_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()+29000}
}
function Run-Case([string]$Name,[string]$Text,$Options,[string]$ExpectedCode,[string]$ExpectedText,[bool]$Truncated,[bool]$LoseFocus=$false,[bool]$Caret=$false){
    $script:selectionCaseSequence+=1;$script:selectionForegroundMismatch=$null
    $casePath=Join-Path $testRoot ('case-{0:d3}.actual.json' -f $script:selectionCaseSequence)
    $case=@{name=$Name;runtime=$Runtime;passed=$false;phase='window_setup';caller_pid=$PID;owned_window=$form.Handle.ToInt64().ToString();expected=@{code=$ExpectedCode;text=$ExpectedText;truncated=$Truncated};synthetic_input=$Text;cli_receipt=$null;actual=$null;failure=$null}
    try{
        $edit.Text=$Text;$form.Show();$form.Activate();$null=$edit.Focus();$edit.Select(0, $(if($Caret){0}else{$Text.Length}))
        [Windows.Forms.Application]::DoEvents()
        if([SelectionEditNative]::GetForegroundWindow() -ne $form.Handle){throw '未获得自建窗口焦点，未调用读取。'}
        Save-Json (Join-Path $testRoot 'target.json') (Target-Context)
        Save-Json (Join-Path $testRoot 'call.json') @{package_id='test.selection-read';action_id='read';action_version=1;input=$Options}
        if($LoseFocus){$other.Show();$other.Activate();[Windows.Forms.Application]::DoEvents();if([SelectionEditNative]::GetForegroundWindow() -ne $other.Handle){throw '失焦前置条件未满足。'}}
        $beforeForeground=[SelectionEditNative]::GetForegroundWindow();$sequence=[SelectionEditNative]::GetClipboardSequenceNumber()
        $script:selectionExpectedForeground=$beforeForeground;$case.phase='invoke_cli'
        $case.cli_receipt=Join-Path $testRoot ('cli-{0:d3}.json' -f ($script:selectionCliSequence+1))
        $reply=Invoke-OwnedCli @('--config',(Join-Path $testRoot 'bindings.json'),'--run-action',(Join-Path $testRoot 'call.json'),'--target-context',(Join-Path $testRoot 'target.json'))
        $case.phase='verify_reply';$case.cli_pid=$reply.pid;$case.actual_exit_code=$reply.code;$case.stdout=$reply.stdout;$case.stderr=$reply.stderr
        if($reply.code -ne 0 -or $reply.stderr){throw "CLI 失败：$Name"}
        $body=$reply.stdout|ConvertFrom-Json;$case.actual=$body
        if($body.status -ne 'succeeded'){throw "动作未成功返回：$Name"}
        $output=$body.output
        if($ExpectedCode){if($output.kind -ne 'error' -or $output.code -ne $ExpectedCode -or $output.effectState -ne 'not_started'){throw "错误契约不符：$Name"}}
        elseif($output.kind -ne 'selection' -or $output.text -cne $ExpectedText -or $output.truncated -ne $Truncated){throw "选区回执不符：$Name"}
        if($edit.Text -cne $Text){throw "自建 Edit 内容被更改：$Name"}
        if([SelectionEditNative]::GetForegroundWindow() -ne $beforeForeground){throw "前台已变化；停止：$Name"}
        if([SelectionEditNative]::GetClipboardSequenceNumber() -ne $sequence){throw "剪贴板序号变化，本项证据受干扰：$Name"}
        $case.passed=$true;$case.phase='complete'
        $results.Add(@{name=$Name;passed=$true;native=$true;foreground_unchanged=$true;clipboard_sequence_unchanged=$true;actual_receipt=$casePath})
    }catch{$case.failure=$_.Exception.Message;throw
    }finally{
        $case.foreground_mismatch=$selectionForegroundMismatch
        if($case.cli_receipt -and (Test-Path -LiteralPath $case.cli_receipt)){$case.cli=Get-Content -LiteralPath $case.cli_receipt -Raw|ConvertFrom-Json}
        Save-Json $casePath $case
        $script:selectionExpectedForeground=[IntPtr]::Zero
    }
}

try{
    if($Runtime -eq 'Node'){[Environment]::SetEnvironmentVariable('OMNITRIGGER_NODE',$RuntimePath,'Process')}
    else{
        [Environment]::SetEnvironmentVariable('OMNITRIGGER_SCRIPT_HOST',$RuntimePath,'Process')
        $description=Invoke-OwnedCli @('--describe-runtime') $RuntimePath
        if($description.code -ne 0 -or $description.stderr){throw '显式脚本宿主描述失败。'}
        $identity=$description.stdout|ConvertFrom-Json
        if($identity.product -ne 'omnitrigger-script-host' -or $identity.engine -ne $Runtime.ToLowerInvariant()){throw '显式宿主引擎与Runtime不一致。'}
    }
    Save-Json (Join-Path $testRoot 'entry-call.json') @{package_id='test.selection-read';action_id='read';action_version=1;input=@{operation='entry_smoke';sentinel='仅合成输入😀'}}
    $preflight=Invoke-OwnedCli @('--config',(Join-Path $testRoot 'bindings.json'),'--run-action',(Join-Path $testRoot 'entry-call.json'))
    if($preflight.code -ne 0 -or $preflight.stderr){throw ('真实CLI入口冒烟失败：'+$preflight.stderr)}
    $entryResult=$preflight.stdout|ConvertFrom-Json
    if($entryResult.status -ne 'succeeded' -or $entryResult.output.kind -ne 'entry_smoke' -or $entryResult.output.sentinel -cne '仅合成输入😀' -or $entryResult.output.hasSelectionRead -ne $true){throw '真实CLI未确认共同入口及选区API，不得继续Native。'}
    Save-Json (Join-Path $testRoot 'ENTRY-RESULT.json') @{passed=$true;runtime=$Runtime;entry_cli_executed=$true;selection_UIA_executed=$false;output=$entryResult.output;app_sha256=(Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash;runtime_sha256=(Get-FileHash -LiteralPath $RuntimePath -Algorithm SHA256).Hash}
    if(-not $RunNative){Write-Output (Join-Path $testRoot 'ENTRY-RESULT.json');return}
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SelectionEditNative {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
  [DllImport("user32.dll")] public static extern uint GetClipboardSequenceNumber();
  public static async System.Threading.Tasks.Task<string> ReadBounded(System.IO.TextReader reader) {
    var result=new System.Text.StringBuilder();var buffer=new char[4096];
    for (;;) { var count=await reader.ReadAsync(buffer,0,buffer.Length).ConfigureAwait(false);if(count==0)return result.ToString();
      if(result.Length+count>65536)throw new System.IO.IOException("测试输出超过64KiB。");result.Append(buffer,0,count); }
  }
}
'@
$form = [Windows.Forms.Form]::new()
$form.Text = 'OmniTrigger 自建 Edit 选区测试 ' + $PID
$form.Width=650; $form.Height=240
$edit = [Windows.Forms.TextBox]::new();$edit.Multiline=$true;$edit.Dock='Fill';$form.Controls.Add($edit)
$other = [Windows.Forms.Form]::new();$other.Text='仅用于失焦拒绝的自有窗口';$other.Width=300;$other.Height=120
$results = [Collections.Generic.List[object]]::new()
$selectionUiReady=$true
    $emoji=[char]::ConvertFromUtf32(0x1f600)
    Run-Case '短选区默认' '合成 A😀' @{} '' '合成 A😀' $false
    Run-Case '128 ASCII 默认' ('A'*128) @{} '' ('A'*128) $false
    Run-Case '129 ASCII 默认拒绝' ('A'*129) @{} 'selection_too_long' '' $false
    Run-Case '129 ASCII 显式截断' ('A'*129) @{overflow='truncate'} '' ('A'*128) $true
    Run-Case '65 emoji 显式截断' ($emoji*65) @{overflow='truncate'} '' ($emoji*64) $true
    Run-Case '截点拆开代理对拒绝' (('A'*127)+$emoji) @{overflow='truncate'} 'invalid_response' '' $false
    Run-Case '空文本选区成功' '' @{} '' '' $false
    Run-Case '非空文本退化选区成功' '未选中的自建文本' @{} '' '' $false $false $true
    Run-Case '原目标失焦拒绝' '原目标仍保留' @{} 'target_changed' '' $false $true

    Save-Json (Join-Path $testRoot 'RESULT.json') @{passed=$true;runtime=$Runtime;entry_cli_executed=$true;selection_UIA_executed=$true;cases=$results;count=$results.Count;automatic_retry=$false}
    Write-Output (Join-Path $testRoot 'RESULT.json')
}catch{
    $selectionRunFailure=$_.Exception.Message
    Save-Json (Join-Path $testRoot 'RESULT.json') @{passed=$false;runtime=$Runtime;selection_UIA_requested=[bool]$RunNative;failure=$selectionRunFailure;completed_cases=$results;failed_case_number=$selectionCaseSequence;automatic_retry=$false}
    throw
}finally{
    $cleanupErrors=[Collections.Generic.List[string]]::new();$formDisposed=$null;$otherDisposed=$null
    try{if($null -ne $other){$other.Dispose();$otherDisposed=$other.IsDisposed}}catch{$cleanupErrors.Add($_.Exception.Message)}
    try{if($null -ne $form){$form.Dispose();$formDisposed=$form.IsDisposed}}catch{$cleanupErrors.Add($_.Exception.Message)}
    $nodeRestored=$false;$scriptRestored=$false
    try{if($null -eq $previousNode){Remove-Item -LiteralPath Env:OMNITRIGGER_NODE -ErrorAction SilentlyContinue}else{[Environment]::SetEnvironmentVariable('OMNITRIGGER_NODE',$previousNode,'Process')};$nodeRestored=[Environment]::GetEnvironmentVariable('OMNITRIGGER_NODE','Process') -ceq $previousNode}catch{$cleanupErrors.Add($_.Exception.Message)}
    try{if($null -eq $previousScript){Remove-Item -LiteralPath Env:OMNITRIGGER_SCRIPT_HOST -ErrorAction SilentlyContinue}else{[Environment]::SetEnvironmentVariable('OMNITRIGGER_SCRIPT_HOST',$previousScript,'Process')};$scriptRestored=[Environment]::GetEnvironmentVariable('OMNITRIGGER_SCRIPT_HOST','Process') -ceq $previousScript}catch{$cleanupErrors.Add($_.Exception.Message)}
    $unconfirmed=@($selectionCliCleanups|Where-Object{-not $_.confirmed}).Count
    $finalConfirmed=$cleanupErrors.Count -eq 0 -and $nodeRestored -and $scriptRestored -and $unconfirmed -eq 0 -and $formDisposed -ne $false -and $otherDisposed -ne $false
    $finalPath=Join-Path $testRoot 'FINAL-CLEANUP.json'
    Save-Json $finalPath @{confirmed=$finalConfirmed;runtime=$Runtime;caller_pid=$PID;failure=$selectionRunFailure;form_disposed=$formDisposed;other_form_disposed=$otherDisposed;node_environment_restored=$nodeRestored;script_environment_restored=$scriptRestored;known_cli_processes=$selectionCliCleanups.ToArray();helper_pid_exposed=$false;foreground_mismatch=$selectionForegroundMismatch;errors=$cleanupErrors.ToArray()}
    if(-not $finalConfirmed){
        Save-Json (Join-Path $testRoot 'RESULT.json') @{passed=$false;runtime=$Runtime;failure='fixture_cleanup_unconfirmed';primary_failure=$selectionRunFailure;cleanup_receipt=$finalPath;automatic_retry=$false}
        throw ('finally清理未确认；不能计通过：'+$finalPath)
    }
}
