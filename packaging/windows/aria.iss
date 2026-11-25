; Inno Setup script for the Windows installer of Aria.
;
; Build the release exe first, then compile this script from the repository root:
;   cargo build --release --locked -p aria-app
;   ISCC.exe /DAppVersion=0.3.3 packaging\windows\aria.iss
; The installer is written to target\installer\Aria-<version>-setup.exe.
;
; The installer is per-user: it needs no administrator rights and installs into
; %LOCALAPPDATA%\Programs\Aria. Uninstalling removes the program files only. Settings, the
; library database, logs and the artwork cache stay where Aria keeps them (see docs/PRIVACY.md),
; and your music files are never touched.

#ifndef AppVersion
  #error Pass the version, e.g. ISCC.exe /DAppVersion=0.3.3 packaging\windows\aria.iss
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
SetupIconFile={#Root}\assets\icon\aria.ico
UninstallDisplayIcon={app}\aria.exe
UninstallDisplayName=Aria {#AppVersion}
CloseApplications=yes
ShowLanguageDialog=auto
; Tells Explorer that "Open with" changed, below.
ChangesAssociations=yes

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

; Aria in "Open with" for the audio files it plays, for this user only. The default program for
; these files is left alone: Windows lets the user choose it ("Open with", "Always"). Uninstalling
; removes what is written here.
[Registry]
Root: HKA; Subkey: "Software\Classes\Aria.AudioFile"; ValueType: string; ValueName: ""; ValueData: "Audio file"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Aria.AudioFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\aria.exe,0"
Root: HKA; Subkey: "Software\Classes\Aria.AudioFile\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\aria.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\aria.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Aria"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\aria.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\aria.exe"" ""%1"""
#define AudioType(Ext) \
  'Root: HKA; Subkey: "Software\Classes\.' + Ext + '\OpenWithProgids"; ValueType: string; ValueName: "Aria.AudioFile"; ValueData: ""; Flags: uninsdeletevalue' + NewLine + \
  'Root: HKA; Subkey: "Software\Classes\Applications\aria.exe\SupportedTypes"; ValueType: string; ValueName: ".' + Ext + '"; ValueData: ""'
{#AudioType("mp3")}
{#AudioType("flac")}
{#AudioType("wav")}
{#AudioType("m4a")}
{#AudioType("aac")}
{#AudioType("ogg")}
{#AudioType("oga")}
{#AudioType("opus")}
{#AudioType("aif")}
{#AudioType("aiff")}

[Run]
Filename: "{app}\aria.exe"; Description: "{cm:LaunchProgram,Aria}"; Flags: nowait postinstall skipifsilent
; "Restart to update" (since 0.3.5): Aria runs this installer silently with /RELAUNCH=1 and quits;
; this starts the new Aria once the files are replaced. Any other silent install starts nothing.
Filename: "{app}\aria.exe"; Flags: nowait skipifnotsilent; Check: Relaunch

; For checking: ISCC /DSavePreprocessed=<file> writes the script as compiled.
#ifdef SavePreprocessed
  #expr SaveToFile(SavePreprocessed)
#endif

; The Pascal code comes last: any line after it, a comment too, would be read as code.
[Code]
function Relaunch: Boolean;
begin
  Result := ExpandConstant('{param:relaunch|0}') = '1';
end;
