; Built by CI: ISCC /DAppVersion=<version> packaging\windows\oxlet.iss, with dist\Oxlet.exe in place.
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{BF9C6154-7345-450C-BACB-638D0016F6BA}
AppName=Oxlet
AppVersion={#AppVersion}
AppPublisher=Dušan Jolović
DefaultDirName={autopf}\Oxlet
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=..\icons\oxlet.ico
UninstallDisplayIcon={app}\Oxlet.exe
OutputDir=..\..
OutputBaseFilename=oxlet-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "..\..\dist\Oxlet.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; The id matches the one Oxlet registers for notifications, so they carry its name and icon.
Name: "{autoprograms}\Oxlet"; Filename: "{app}\Oxlet.exe"; AppUserModelID: "dev.oxlet.app"
Name: "{autodesktop}\Oxlet"; Filename: "{app}\Oxlet.exe"; AppUserModelID: "dev.oxlet.app"; Tasks: desktopicon

[Run]
Filename: "{app}\Oxlet.exe"; Description: "{cm:LaunchProgram,Oxlet}"; Flags: nowait postinstall skipifsilent
