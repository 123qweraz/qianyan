#define MyAppName "Rust IME"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Rust-IME Team"
#define MyAppExeName "rust-ime.exe"
#define MyAppDllName "rust_ime_tsf_v3.dll"

[Setup]
AppId={{A7F23B4C-D5E6-4F7G-8H9I-J0K1L2M3N4O5}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
AllowNoIcons=yes
OutputBaseFilename=RustIME_Setup
Compression=lzma
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=admin
SetupIconFile=picture\rust-ime_v2.ico

[Languages]
Name: "chinesesimplified"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "startup"; Description: "开机自动启动后台程序"; GroupDescription: "其他任务:";

[Files]
Source: "target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\{#MyAppDllName}"; DestDir: "{app}"; Flags: ignoreversion
; 包含所有资源目录
Source: "data\*"; DestDir: "{app}\data"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "dicts\*"; DestDir: "{app}\dicts"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "static\*"; DestDir: "{app}\static"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "configs\*"; DestDir: "{app}\configs"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "fonts\*"; DestDir: "{app}\fonts"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "sounds\*"; DestDir: "{app}\sounds"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "picture\*"; DestDir: "{app}\picture"; Flags: ignoreversion recursesubdirs createallsubdirs

[Dirs]
; 关键：允许普通用户在这些目录下写入数据 (保存配置和词库)
Name: "{app}\data"; Permissions: users-full
Name: "{app}\configs"; Permissions: users-full

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "RustIME"; ValueData: """{app}\{#MyAppExeName}"""; Flags: uninsdeletevalue; Tasks: startup

[Run]
; 注册 TSF DLL
Filename: "regsvr32.exe"; Parameters: "/s ""{app}\{#MyAppDllName}"""; StatusMsg: "正在向系统注册输入法组件..."; Flags: runhidden
; 安装后启动
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "taskkill.exe"; Parameters: "/f /im {#MyAppExeName}"; RunOnceId: "StopApp"; Flags: runhidden
Filename: "regsvr32.exe"; Parameters: "/u /s ""{app}\{#MyAppDllName}"""; StatusMsg: "正在注销输入法组件..."; Flags: runhidden
