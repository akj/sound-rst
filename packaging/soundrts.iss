; The Windows installer, for Inno Setup 6. It installs the folder that
; scripts/stage-release.ps1 makes, for the current user only, so it asks for
; no administrator password.
;
;   iscc /DVersion=0.1.0 /DSource=C:\full\path\dist\soundrts-0.1.0-windows-x64 ^
;        /DOutputDir=C:\full\path\dist /DOutputName=soundrts-0.1.0-windows-x64-setup ^
;        packaging\soundrts.iss
;
; The uninstaller removes what the installer put there. The player's settings
; and the original game's data, in %APPDATA%\SoundRTS, stay.

#ifndef Version
  #error Pass /DVersion
#endif
#ifndef Source
  #error Pass /DSource
#endif
#ifndef OutputDir
  #error Pass /DOutputDir
#endif
#ifndef OutputName
  #error Pass /DOutputName
#endif

; The name in the Start menu and in the list of installed programs.
; scripts/check-release.ps1 looks for the Start menu entry by this name.
#define Name "sound-rst"

[Setup]
; Windows knows an installed program by this id: a newer installer with the
; same id updates the installed game in place. Never change it.
AppId={{0A196EA5-31D6-4E52-A6AE-C3F2A7C6B13E}
AppName={#Name}
AppVersion={#Version}
AppPublisherURL=https://github.com/akj/sound-rst
AppSupportURL=https://github.com/akj/sound-rst/issues
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#Name}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
Compression=lzma2
SolidCompression=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
; The data folder comes whole with each version: an update removes the
; files that the new version no longer has.
Type: filesandordirs; Name: "{app}\data"

[Files]
Source: "{#Source}\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{autoprograms}\{#Name}"; Filename: "{app}\soundrts.exe"
Name: "{autodesktop}\{#Name}"; Filename: "{app}\soundrts.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\soundrts.exe"; Description: "{cm:LaunchProgram,{#Name}}"; Flags: nowait postinstall skipifsilent
