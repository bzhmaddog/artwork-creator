; The Windows installer (Inno Setup 6): the executable and its templates folder, a Start
; menu entry, an optional desktop shortcut, .artwork files opened with the app, and an
; uninstaller. The settings and the user's templates, in %APPDATA%\Artwork Creator, are
; left in place on uninstall.
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

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

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
Filename: "{app}\artwork-creator.exe"; Description: "{cm:LaunchProgram,Artwork Creator}"; Flags: nowait postinstall skipifsilent
