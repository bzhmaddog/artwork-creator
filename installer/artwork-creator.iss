; The Windows installer (Inno Setup 6): the executable and its templates folder, a Start
; menu entry, an optional desktop shortcut, .artwork files opened with the app, and an
; uninstaller. The settings and the user's templates, in %APPDATA%\Artwork Creator, are
; left in place on uninstall.
;
; When no Python 3 is installed, it offers to install it, for the Python plugins: the
; python.org installer, downloaded during setup and checked against its SHA-256. Declined
; or failing, the app is installed without it and the Python plugins are not available.
; Python is an app of its own: it stays installed when Artwork Creator is removed.
;
; This file is UTF-8 with a BOM, for the French texts.
;
; The Release workflow compiles it after the release build, from the repository root:
;   iscc /DAppVersion=X.Y.Z installer\artwork-creator.iss
; which writes dist\artwork-creator-windows-setup.exe. /DExe=... names another
; executable, relative to this folder; `make windows-setup` passes the MinGW build
; (scripts/windows-setup.sh, Inno Setup in a container).

#ifndef AppVersion
  #error Pass /DAppVersion=X.Y.Z
#endif
#ifndef Exe
  #define Exe "..\target\release\artwork-creator.exe"
#endif

; The Python offered when none is installed, at a pinned version checked against its
; SHA-256 (from https://www.python.org/downloads/windows/); raise both on purpose.
#define PythonVersion "3.14.10"
#define PythonSha256 "92480239264744fe0386a1bd8275786fefa126e3129e56a68e22b52c73329965"
#if PythonVersion == "" || PythonSha256 == ""
  #error Set PythonVersion and PythonSha256
#endif
#define PythonInstaller "python-" + PythonVersion + "-amd64.exe"
#define PythonUrl "https://www.python.org/ftp/python/" + PythonVersion + "/" + PythonInstaller

[Setup]
; Identifies the app for upgrades and the uninstaller: never change it.
AppId={{0C79B7C1-49C7-4279-867D-7DF6863343FF}
AppName=Artwork Creator
AppVersion={#AppVersion}
AppPublisher=bzhmaddog
AppPublisherURL=https://github.com/bzhmaddog/artwork-creator
DefaultDirName={autopf}\Artwork Creator
DefaultGroupName=Artwork Creator
DisableProgramGroupPage=yes
; Installs for all users (admin) or, if chosen in the first dialog, for the current user only.
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\LICENSE
SetupIconFile=..\assets\icon\artwork-creator.ico
UninstallDisplayIcon={app}\artwork-creator.exe
ChangesAssociations=yes
OutputDir=..\dist
OutputBaseFilename=artwork-creator-windows-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"

[CustomMessages]
en.InstallPython=Install Python (needed for Python plugins)
fr.InstallPython=Installer Python (nécessaire pour les plugins Python)
en.InstallingPython=Installing Python...
fr.InstallingPython=Installation de Python...
en.PythonNotDownloaded=Python could not be downloaded: %1%n%nArtwork Creator will be installed without it, and Python plugins won't be available.
fr.PythonNotDownloaded=Python n'a pas pu être téléchargé : %1%n%nArtwork Creator sera installé sans lui, et les plugins Python ne seront pas disponibles.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
; Shown only when no Python 3 is installed; checked.
Name: "python"; Description: "{cm:InstallPython}"; Check: not PythonFound

[Files]
Source: "{#Exe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\templates\*"; DestDir: "{app}\templates"; Flags: ignoreversion recursesubdirs createallsubdirs

; Templates removed from a new version go away on upgrade.
[InstallDelete]
Type: filesandordirs; Name: "{app}\templates"

[Icons]
Name: "{group}\Artwork Creator"; Filename: "{app}\artwork-creator.exe"
Name: "{autodesktop}\Artwork Creator"; Filename: "{app}\artwork-creator.exe"; Tasks: desktopicon

[Registry]
; .artwork project files open with the app.
Root: HKA; Subkey: "Software\Classes\.artwork\OpenWithProgids"; ValueType: string; ValueName: "ArtworkCreator.Project"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\ArtworkCreator.Project"; ValueType: string; ValueName: ""; ValueData: "Artwork Creator project"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\ArtworkCreator.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\artwork-creator.exe,0"
Root: HKA; Subkey: "Software\Classes\ArtworkCreator.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\artwork-creator.exe"" ""%1"""

[Run]
; Before the app is started: the app looks for Python when it starts.
Filename: "{tmp}\{#PythonInstaller}"; Parameters: "/quiet {code:PythonScope} PrependPath=1 Include_launcher=1 Include_test=0"; StatusMsg: "{cm:InstallingPython}"; Tasks: python; Check: PythonDownloaded
Filename: "{app}\artwork-creator.exe"; Description: "{cm:LaunchProgram,Artwork Creator}"; Flags: nowait postinstall skipifsilent

[Code]
var
  PythonPage: TDownloadWizardPage;
  Downloaded: Boolean;

{ Whether a Python 3 is registered under Root (PEP 514: python.org and Store installs). }
function HasPython3(Root: Integer): Boolean;
var
  Versions: TArrayOfString;
  I: Integer;
begin
  Result := False;
  if RegGetSubkeyNames(Root, 'Software\Python\PythonCore', Versions) then
    for I := 0 to GetArrayLength(Versions) - 1 do
      if Copy(Versions[I], 1, 2) = '3.' then
        Result := True;
end;

function PythonFound: Boolean;
begin
  Result := HasPython3(HKCU) or HasPython3(HKLM64) or HasPython3(HKLM32);
end;

function PythonDownloaded: Boolean;
begin
  Result := Downloaded;
end;

{ Python for the same users as the app. }
function PythonScope(Param: String): String;
begin
  if IsAdminInstallMode then
    Result := 'InstallAllUsers=1'
  else
    Result := 'InstallAllUsers=0 InstallLauncherAllUsers=0';
end;

procedure InitializeWizard;
begin
  PythonPage := CreateDownloadPage(SetupMessage(msgWizardPreparing), SetupMessage(msgPreparingDesc), nil);
end;

{ Downloads Python after the Ready page when its task is checked. A failed download
  is reported and the install goes on without it. }
function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = wpReady) and WizardIsTaskSelected('python') then begin
    PythonPage.Clear;
    PythonPage.Add('{#PythonUrl}', '{#PythonInstaller}', '{#PythonSha256}');
    PythonPage.Show;
    try
      try
        PythonPage.Download;
        Downloaded := True;
      except
        if not PythonPage.AbortedByUser then
          SuppressibleMsgBox(FmtMessage(CustomMessage('PythonNotDownloaded'), [GetExceptionMessage]), mbError, MB_OK, IDOK);
      end;
    finally
      PythonPage.Hide;
    end;
  end;
end;
