; Built by the Release workflow: ISCC /DAppVersion=x.y.z /DOutputName=name installer\setup.iss
[Setup]
AppId={{C88337C0-A5D0-4BB8-834A-B04F834DD3B0}
AppName=Bolt Explorer
AppVersion={#AppVersion}
AppPublisher=MyDrift
AppPublisherURL=https://github.com/MyDrift-user/bolt-explorer
AppSupportURL=https://github.com/MyDrift-user/bolt-explorer/issues
DefaultDirName={autopf}\Bolt Explorer
DefaultGroupName=Bolt Explorer
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename={#OutputName}
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\bolt-explorer.exe
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequiredOverridesAllowed=dialog
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\bolt-explorer.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Bolt Explorer"; Filename: "{app}\bolt-explorer.exe"
Name: "{autodesktop}\Bolt Explorer"; Filename: "{app}\bolt-explorer.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\bolt-explorer.exe"; Description: "{cm:LaunchProgram,Bolt Explorer}"; Flags: nowait postinstall skipifsilent
