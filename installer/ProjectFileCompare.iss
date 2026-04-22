#define MyAppName "Project File Compare"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "FYX"
#define MyAppExeName "ProjectFileCompare.exe"
#define MyAppId "{{C8A34A8F-70C3-4AC6-9D1A-CE2399182FD2}"

#ifndef MyAppArch
  #define MyAppArch "x64"
#endif

#ifndef MyOutputBaseFilename
  #define MyOutputBaseFilename "ProjectFileCompare-Setup-" + MyAppArch
#endif

#if MyAppArch == "x86"
  #define MyArchitecturesAllowed "x86compatible"
  #define MyArchitecturesInstallIn64BitMode ""
#elif MyAppArch == "x64"
  #define MyArchitecturesAllowed "x64compatible"
  #define MyArchitecturesInstallIn64BitMode "x64compatible"
#elif MyAppArch == "arm64"
  #define MyArchitecturesAllowed "arm64"
  #define MyArchitecturesInstallIn64BitMode "arm64"
#else
  #error Unsupported MyAppArch value
#endif

[Setup]
AppId={#MyAppId}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={localappdata}\Programs\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed={#MyArchitecturesAllowed}
#if "" != MyArchitecturesInstallIn64BitMode
ArchitecturesInstallIn64BitMode={#MyArchitecturesInstallIn64BitMode}
#endif
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
OutputDir=Output
OutputBaseFilename={#MyOutputBaseFilename}
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "chinesesimp"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加任务:"; Flags: unchecked

[Files]
Source: "..\dist\ProjectFileCompare.exe"; DestDir: "{app}"; Flags: ignoreversion

[Dirs]
Name: "{localappdata}\ProjectFileCompare"

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "启动 {#MyAppName}"; Flags: nowait postinstall skipifsilent
