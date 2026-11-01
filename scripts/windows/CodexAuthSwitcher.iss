; Packaging behavior reference: ramag-platform@59399356bc88f215c8eacca632dcb3e13953632a.
; This Inno Setup definition is independently adapted for Codex Auth Switcher.
#define AppName "Codex Auth Switcher"
#define AppId "com.bruceblink.codex-auth-switcher"
#define AppPublisher "bruceblink"
#define AppRepository "https://github.com/bruceblink/codex-auth-switcher"
#define AppExeName "codex-auth-switcher.exe"
#define RepoRoot AddBackslash(SourcePath) + "..\..\"
#define SourceExe GetEnv("CAS_PACKAGE_EXE")
#define AppVersion GetEnv("CAS_PACKAGE_VERSION")
#define AppVersionInfo GetEnv("CAS_PACKAGE_VERSION_INFO")
#define StageDir GetEnv("CAS_PACKAGE_STAGE_DIR")

#if SourceExe == ""
  #error CAS_PACKAGE_EXE is required
#endif
#if AppVersion == ""
  #error CAS_PACKAGE_VERSION is required
#endif
#if AppVersionInfo == ""
  #error CAS_PACKAGE_VERSION_INFO is required
#endif
#if StageDir == ""
  #error CAS_PACKAGE_STAGE_DIR is required
#endif

[Setup]
AppId={#AppId}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppRepository}
AppSupportURL={#AppRepository}/issues
AppUpdatesURL={#AppRepository}/releases
DefaultDirName={localappdata}\Programs\Codex Auth Switcher
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile={#RepoRoot}LICENSE
SetupIconFile={#RepoRoot}crates\switcher-desktop\assets\codex-auth-switcher.ico
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName}
VersionInfoVersion={#AppVersionInfo}
VersionInfoCompany={#AppPublisher}
VersionInfoDescription={#AppName} installer
VersionInfoProductName={#AppName}
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimplified"; MessagesFile: "languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "{#AppExeName}"; Flags: ignoreversion
Source: "{#RepoRoot}LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}THIRD-PARTY-NOTICES.html"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\SOURCES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}scripts\windows\languages\LICENSE.txt"; DestDir: "{app}"; DestName: "LICENSE-Inno-Chinese-Translation.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(AppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
