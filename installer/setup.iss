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
UninstallDisplayName=Daleris
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequiredOverridesAllowed=dialog
WizardStyle=modern
; Setup speaks Windows's display language when it's one of the three below, English otherwise, without asking.
ShowLanguageDialog=no
LanguageDetectionMethod=uilanguage

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"
Name: "it"; MessagesFile: "compiler:Languages\Italian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\daleris.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\daleris-open.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\daleris-dialogs-x64.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\daleris-dialogs-x86.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "machine-install.txt"; DestDir: "{app}"; Flags: ignoreversion; Check: IsAdminInstallMode

[Icons]
Name: "{autoprograms}\Daleris"; Filename: "{app}\daleris.exe"; AppUserModelID: "MyDrift.Daleris"
Name: "{autodesktop}\Daleris"; Filename: "{app}\daleris.exe"; AppUserModelID: "MyDrift.Daleris"; Tasks: desktopicon

[Run]
Filename: "{app}\daleris.exe"; Description: "{cm:LaunchProgram,Daleris}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Folders go back to File Explorer for whoever uninstalls. Anyone else gets the same from the launcher. See src/integration_windows.rs.
Filename: "{app}\daleris.exe"; Parameters: "--restore-explorer"; Flags: runhidden; RunOnceId: "RestoreExplorer"
Filename: "{sys}\schtasks.exe"; Parameters: "/Delete /F /TN ""Daleris Updater"""; Flags: runhidden; Check: IsAdminInstallMode; RunOnceId: "DeleteUpdateTask"

[UninstallDelete]
Type: filesandordirs; Name: "{app}\updates"

[Code]
// For a PC-wide install: a task installs updates as SYSTEM, so nobody gets an admin prompt. See src/update.rs.
// It runs this folder's program as SYSTEM, so only admins may change it, wherever it was installed.
// There's no task if that can't be done. Only on the folder, as files inherit from it: with /T the same command
// leaves files with no permissions.
procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
begin
  if (CurStep = ssPostInstall) and IsAdminInstallMode then
  begin
    if Exec(ExpandConstant('{sys}\icacls.exe'), '"' + ExpandConstant('{app}') + '" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F *S-1-5-32-545:(OI)(CI)RX /C', '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0) then
      Exec(ExpandConstant('{sys}\schtasks.exe'), '/Create /F /TN "Daleris Updater" /SC HOURLY /RU SYSTEM /RL HIGHEST /TR "\"' + ExpandConstant('{app}') + '\daleris.exe\" --update-service"', '', SW_HIDE, ewWaitUntilTerminated, Code)
    else
      Log('Could not lock the install folder, so there is no update task');
  end;
end;
