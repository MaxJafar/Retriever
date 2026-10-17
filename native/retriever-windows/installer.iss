#ifndef Payload
  #error Payload must point to the built native candidate
#endif
#ifndef Output
  #error Output must point to the candidate package directory
#endif
#ifndef Version
  #error Version must match the native application and engine
#endif
[Setup]
AppId={{08D3E77C-793B-47E1-98D7-4FCE46FF7358}
AppName=Retriever
AppVersion={#Version}
AppPublisher=Retriever contributors
DefaultDirName={localappdata}\Programs\Retriever\{#Version}
UsePreviousAppDir=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
OutputDir={#Output}
OutputBaseFilename=Retriever-{#Version}-windows-x64-setup
SetupIconFile=Retriever.ico
UninstallDisplayIcon={app}\Retriever.ico
WizardStyle=modern
Compression=lzma2/fast
SolidCompression=yes
CloseApplications=no
RestartApplications=no
DisableProgramGroupPage=yes
LicenseFile=..\..\LICENSE
InfoBeforeFile=..\..\release\PREVIEW-NOTICE.txt
[Files]
Source: "{#Payload}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion
[Icons]
Name: "{userprograms}\Retriever"; Filename: "{app}\Retriever.exe"; IconFilename: "{app}\Retriever.ico"
Name: "{userprograms}\Retriever Quick Search"; Filename: "{app}\Retriever.exe"; Parameters: "--quick"; IconFilename: "{app}\Retriever.ico"
[Run]
Filename: "{app}\Retriever.exe"; Description: "Open Retriever"; Flags: nowait postinstall skipifsilent
; Versioned app folders retain rollback files. State and models live outside them.
[Code]
procedure CurStepChanged(CurStep: TSetupStep);
var Code: Integer;
begin
  if CurStep = ssPostInstall then
    if not Exec(ExpandConstant('{app}\Retriever.exe'), '--upgrade-folder-integration', '', SW_HIDE, ewWaitUntilTerminated, Code) or (Code <> 0) then
      MsgBox('Retriever is installed. Folder integration could not be updated; the previous app and its restore record are retained. Run this installer again before removing the previous app.', mbInformation, MB_OK);
end;

function InitializeUninstall(): Boolean;
var Code: Integer;
begin
  Result := Exec(ExpandConstant('{app}\Retriever.exe'), '--restore-folder-default', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := Result and (Code = 0);
  if not Result then MsgBox('Restore folder integration in Retriever before uninstalling. The app has been kept so folder opening continues to work.', mbError, MB_OK);
end;
