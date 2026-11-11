; Inno Setup script for the Windows installer of Aria.
;
; Build the release exe first, then compile this script from the repository root:
;   cargo build --release --locked -p aria-app
;   ISCC.exe /DAppVersion=0.2.1 packaging\windows\aria.iss
; The installer is written to target\installer\Aria-<version>-setup.exe.
;
; The installer is per-user: it needs no administrator rights and installs into
; %LOCALAPPDATA%\Programs\Aria. Uninstalling removes the program files only. Settings, the
; library database, logs and the artwork cache stay where Aria keeps them (see docs/PRIVACY.md),
; and your music files are never touched.

#ifndef AppVersion
  #error Pass the version, e.g. ISCC.exe /DAppVersion=0.2.1 packaging\windows\aria.iss
#endif

#define Root "..\.."

[Setup]
; A fixed id, so a later version upgrades this install instead of adding a second one.
AppId={{6F0B8C52-3E1D-4B7A-9C55-2A8E4D1F7B30}
AppName=Aria
AppVersion={#AppVersion}
AppVerName=Aria {#AppVersion}
AppPublisher=Aria contributors
AppPublisherURL=https://github.com/Hixenov/Aria
AppSupportURL=https://github.com/Hixenov/Aria/issues
AppUpdatesURL=https://github.com/Hixenov/Aria/releases
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=
DefaultDirName={autopf}\Aria
DefaultGroupName=Aria
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#Root}\target\installer
OutputBaseFilename=Aria-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\aria.exe
UninstallDisplayName=Aria {#AppVersion}
CloseApplications=yes
ShowLanguageDialog=auto

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "hy"; MessagesFile: "compiler:Languages\Armenian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Root}\target\release\aria.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\LICENSE-MIT"; DestDir: "{app}"; DestName: "LICENSE-MIT.txt"; Flags: ignoreversion
Source: "{#Root}\LICENSE-APACHE"; DestDir: "{app}"; DestName: "LICENSE-APACHE.txt"; Flags: ignoreversion
Source: "{#Root}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Aria"; Filename: "{app}\aria.exe"
Name: "{group}\{cm:UninstallProgram,Aria}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Aria"; Filename: "{app}\aria.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\aria.exe"; Description: "{cm:LaunchProgram,Aria}"; Flags: nowait postinstall skipifsilent
