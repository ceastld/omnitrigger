[Setup]
AppId=OmniTrigger
AppName=OmniTrigger
AppVersion={#AppVersion}
AppPublisher=OmniTrigger
AppPublisherURL=https://github.com/ceastld/omnitrigger
DefaultDirName={localappdata}\Programs\OmniTrigger
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
DisableDirPage=yes
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\omnitrigger.exe
SetupIconFile=..\crates\omnitrigger-app\assets\omnitrigger.ico
OutputBaseFilename=OmniTrigger-{#AppVersion}-windows-x86_64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "{#Payload}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs
Source: "compiler:license.txt"; DestDir: "{app}\licenses\inno-setup"

[Icons]
Name: "{userprograms}\OmniTrigger\OmniTrigger"; Filename: "{app}\omnitrigger.exe"
Name: "{userprograms}\OmniTrigger\OmniTrigger 测试"; Filename: "{app}\omnitrigger.exe"; Parameters: "--test-workspace"

[Run]
Filename: "{app}\omnitrigger.exe"; Description: "启动 OmniTrigger"; Flags: postinstall nowait skipifsilent
