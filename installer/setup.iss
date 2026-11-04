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

[Icons]
Name: "{autoprograms}\Daleris"; Filename: "{app}\daleris.exe"
Name: "{autodesktop}\Daleris"; Filename: "{app}\daleris.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\daleris.exe"; Description: "{cm:LaunchProgram,Daleris}"; Flags: nowait postinstall skipifsilent
