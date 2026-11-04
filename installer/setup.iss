; Built by the Release workflow: ISCC /DAppVersion=x.y.z /DOutputName=name installer\setup.iss
[Setup]
AppId={{C88337C0-A5D0-4BB8-834A-B04F834DD3B0}
AppName=Daleris
AppVersion={#AppVersion}
AppPublisher=MyDrift
AppPublisherURL=https://github.com/MyDrift-user/daleris
AppSupportURL=https://github.com/MyDrift-user/daleris/issues
DefaultDirName={autopf}\Daleris
DefaultGroupName=Daleris
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename={#OutputName}
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\daleris.exe
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequiredOverridesAllowed=dialog
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\daleris.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Daleris"; Filename: "{app}\daleris.exe"; AppUserModelID: "MyDrift.Daleris"
Name: "{autodesktop}\Daleris"; Filename: "{app}\daleris.exe"; AppUserModelID: "MyDrift.Daleris"; Tasks: desktopicon

[Run]
; For a PC-wide install: installs updates as SYSTEM, so nobody gets an admin prompt. See src/update.rs.
Filename: "{sys}\schtasks.exe"; Parameters: "/Create /F /TN ""Daleris Updater"" /SC HOURLY /RU SYSTEM /RL HIGHEST /TR ""\""{app}\daleris.exe\"" --update-service"""; Flags: runhidden; Check: IsAdminInstallMode
Filename: "{app}\daleris.exe"; Description: "{cm:LaunchProgram,Daleris}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\schtasks.exe"; Parameters: "/Delete /F /TN ""Daleris Updater"""; Flags: runhidden; Check: IsAdminInstallMode; RunOnceId: "DeleteUpdateTask"

[UninstallDelete]
Type: filesandordirs; Name: "{app}\updates"
