; Inno Setup script for the Windows installer. Built by .github/workflows/release.yml:
;   ISCC /DAppVersion=1.0.0-beta.3 installer\tonestrand.iss     (output: installer\Output\)
; It installs for the current user only (no administrator rights) and adds an uninstaller.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{7F3C2B1E-5A64-4D0B-9E2A-3C8B1F6D4A10}
AppName=ToneStrand
AppVersion={#AppVersion}
AppPublisher=ToneStrand
AppPublisherURL=https://github.com/radioactivetoy/ToneStrand
DefaultDirName={autopf}\ToneStrand
DefaultGroupName=ToneStrand
UninstallDisplayIcon={app}\tonestrand.exe
OutputDir=Output
OutputBaseFilename=ToneStrand-Setup-{#AppVersion}
LicenseFile=..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesInstallIn64BitMode=x64compatible

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "..\target\release\tonestrand.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\ToneStrand"; Filename: "{app}\tonestrand.exe"
Name: "{autodesktop}\ToneStrand"; Filename: "{app}\tonestrand.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\tonestrand.exe"; Description: "Start ToneStrand"; Flags: nowait postinstall skipifsilent
