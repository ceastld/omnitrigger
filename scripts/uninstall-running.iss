// Windows Restart Manager 按进程和创建时间请求正常退出。只登记当前安装路径，
// 避免影响用户在其他目录运行的同名程序或正在读取安装文件的其他应用。
type
  TRmProcess = record
    Pid: DWORD;
    Created: TFileTime;
  end;

function EnumProcesses(var FirstPid: DWORD; Capacity: DWORD; var Used: DWORD): BOOL;
  external 'K32EnumProcesses@kernel32.dll stdcall uninstallonly';
function OpenProcess(Access: DWORD; Inherit: BOOL; Pid: DWORD): THandle;
  external 'OpenProcess@kernel32.dll stdcall uninstallonly';
function QueryImage(Process: THandle; Flags: DWORD; Buffer: String; var Length: DWORD): BOOL;
  external 'QueryFullProcessImageNameW@kernel32.dll stdcall uninstallonly';
function GetProcessTimes(Process: THandle; var Created, Exited, Kernel, User: TFileTime): BOOL;
  external 'GetProcessTimes@kernel32.dll stdcall uninstallonly';
function CloseHandle(Handle: THandle): BOOL;
  external 'CloseHandle@kernel32.dll stdcall uninstallonly';
function OpenImageForDelete(Name: String; Access, Share: DWORD; Security: DWORD_PTR;
  Disposition, Flags: DWORD; Template: THandle): THandle;
  external 'CreateFileW@kernel32.dll stdcall uninstallonly';
function RmStartSession(var Session: DWORD; Flags: DWORD; Key: String): DWORD;
  external 'RmStartSession@rstrtmgr.dll stdcall uninstallonly';
function RmRegisterResources(Session, FileCount: DWORD; Files: DWORD_PTR;
  AppCount: DWORD; var App: TRmProcess; ServiceCount: DWORD; Services: DWORD_PTR): DWORD;
  external 'RmRegisterResources@rstrtmgr.dll stdcall uninstallonly';
function RmShutdown(Session, Flags: DWORD; Callback: DWORD_PTR): DWORD;
  external 'RmShutdown@rstrtmgr.dll stdcall uninstallonly';
function RmEndSession(Session: DWORD): DWORD;
  external 'RmEndSession@rstrtmgr.dll stdcall uninstallonly';

procedure CheckUninstallStatus(Status: DWORD);
var
  Message: String;
begin
  if Status = 0 then Exit;
  Message := '无法移除安装版。请关闭占用安装文件的程序后再卸载。' + #13#10 +
    SysErrorMessage(Status);
  Log(Message);
  SuppressibleMsgBox(Message, mbError, MB_OK, IDOK);
  Abort;
end;

function InstalledProcess(Pid: DWORD; const Expected: String; var App: TRmProcess): Boolean;
var
  Handle: THandle;
  Image: String;
  Length: DWORD;
  Exited, Kernel, User: TFileTime;
begin
  Result := False;
  Handle := OpenProcess($1000, False, Pid); // PROCESS_QUERY_LIMITED_INFORMATION
  if Handle = 0 then Exit;
  try
    Length := 32768;
    SetLength(Image, Length);
    if not QueryImage(Handle, 0, Image, Length) then Exit;
    SetLength(Image, Length);
    if CompareText(Image, Expected) <> 0 then Exit;
    App.Pid := Pid;
    if not GetProcessTimes(Handle, App.Created, Exited, Kernel, User) then
      CheckUninstallStatus(DLLGetLastError);
    Result := True;
  finally
    CloseHandle(Handle);
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Pids: array of DWORD;
  Used, Session: DWORD;
  Image: THandle;
  I: Integer;
  Started: Boolean;
  App: TRmProcess;
  Expected, Key: String;
begin
  // 此阶段位于用户确认之后、卸载器开始删除文件之前。
  if CurUninstallStep <> usUninstall then Exit;
  Expected := ExpandConstant('{app}\omnitrigger.exe');
  if not FileExists(Expected) then Exit;
  SetArrayLength(Pids, 256);
  repeat
    if not EnumProcesses(Pids[0], GetArrayLength(Pids) * 4, Used) then CheckUninstallStatus(DLLGetLastError);
    if Used < DWORD(GetArrayLength(Pids)) * 4 then Break;
    SetArrayLength(Pids, GetArrayLength(Pids) * 2);
  until False;
  Started := False;
  try
    for I := 0 to Integer(Used div 4) - 1 do begin
      if InstalledProcess(Pids[I], Expected, App) then begin
        if not Started then begin
          SetLength(Key, 33); // CCH_RM_SESSION_KEY + 1
          CheckUninstallStatus(RmStartSession(Session, 0, Key));
          Started := True;
        end;
        CheckUninstallStatus(RmRegisterResources(Session, 0, 0, 1, App, 0, 0));
      end;
    end;
    if Started then CheckUninstallStatus(RmShutdown(Session, 0, 0));
  finally
    if Started then RmEndSession(Session);
  end;
  // 其他程序可能仍以不共享删除的方式打开 EXE。保留卸载登记，让用户解除占用后重试。
  Image := OpenImageForDelete(Expected, $10000, 7, 0, 3, 0, 0); // DELETE, share all, OPEN_EXISTING
  if Image = THandle(-1) then CheckUninstallStatus(DLLGetLastError)
  else CloseHandle(Image);
end;
