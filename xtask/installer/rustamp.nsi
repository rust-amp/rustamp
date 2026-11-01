; The Rustamp installer. Built by `cargo xtask package --installer`, which passes
; VERSION, PLAYER_DIR, HOST_DIR and OUTFILE (and SIGN_UNINSTALLER with a certificate).
;
; Per user, no administrator rights. The configuration in %APPDATA%\Rustamp is
; never touched, so an upgrade and an uninstall both keep it. Whatever the user
; added to the install folder (a Plugins folder) also stays: only the files
; listed here are removed.

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user
ManifestDPIAware true

!define PRODUCT "Rustamp"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Rustamp"

Name "${PRODUCT} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\Rustamp"
; An upgrade goes to the folder the last install chose.
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
; A file in use (the player is running) stops the install: no half upgrade.
AllowSkipFiles off

!ifdef SIGN_UNINSTALLER
  !uninstfinalize '"${SIGN_UNINSTALLER}" package --sign-file "%1"' = 0
!endif

!include "MUI2.nsh"

!define MUI_FINISHPAGE_RUN "$INSTDIR\rustamp.exe"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section
  SetOutPath "$INSTDIR"
  File "${PLAYER_DIR}\rustamp.exe"
  File "${PLAYER_DIR}\rustamp-dechost-symphonia.exe"
  File "${PLAYER_DIR}\rustamp-dsphost-stages.exe"
  File "${PLAYER_DIR}\rustamp-plughost32.exe"
  File "${PLAYER_DIR}\LICENSE"
  File "${PLAYER_DIR}\PLUGIN-EXCEPTION.md"
  File "${PLAYER_DIR}\THIRD_PARTY.md"
  File "${PLAYER_DIR}\QUICKSTART.md"
  File "${PLAYER_DIR}\manifest.json"

  ; The plug-in host beside the player carries its own licence.
  SetOutPath "$INSTDIR\licenses\plughost"
  File "${HOST_DIR}\LICENSE-MIT"
  File "${HOST_DIR}\LICENSE-APACHE"
  File "${HOST_DIR}\THIRD_PARTY.md"

  SetOutPath "$INSTDIR"
  CreateShortcut "$SMPROGRAMS\Rustamp.lnk" "$INSTDIR\rustamp.exe"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "The Rustamp contributors"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "https://github.com/rust-amp/rustamp"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\rustamp.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  ; The player first: while it runs its file cannot go, and nothing else has yet.
  Delete "$INSTDIR\rustamp.exe"
  IfFileExists "$INSTDIR\rustamp.exe" 0 +2
    Abort "Rustamp is running. Close it and uninstall again."

  Delete "$INSTDIR\rustamp-dechost-symphonia.exe"
  Delete "$INSTDIR\rustamp-dsphost-stages.exe"
  Delete "$INSTDIR\rustamp-plughost32.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\PLUGIN-EXCEPTION.md"
  Delete "$INSTDIR\THIRD_PARTY.md"
  Delete "$INSTDIR\QUICKSTART.md"
  Delete "$INSTDIR\manifest.json"
  Delete "$INSTDIR\licenses\plughost\LICENSE-MIT"
  Delete "$INSTDIR\licenses\plughost\LICENSE-APACHE"
  Delete "$INSTDIR\licenses\plughost\THIRD_PARTY.md"
  RMDir "$INSTDIR\licenses\plughost"
  RMDir "$INSTDIR\licenses"
  Delete "$SMPROGRAMS\Rustamp.lnk"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"
SectionEnd
