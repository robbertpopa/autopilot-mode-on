; Autopilot Mode On installer for Windows. Build with:
;   makensis /DVERSION=0.1.0 /DEXE=path\to\autopilot-ui.exe /DGUARDIAN=path\to\autopilot-process-guardian.exe /DPRACTICE=path\to\autopilot-fake-agent.exe /DICON=path\to\icon.ico /DOUT=autopilot-mode-on-setup-x64.exe installer.nsi
; The window starts agents through the guardian and the practice agent, and
; looks for both next to itself, so they ship in the same folder.
;
; It installs for the current user only, so it never asks for administrator
; rights and an in-app update can run it without a prompt. Running it again
; over an existing install upgrades in place.

Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define NAME "Autopilot Mode On"
!define KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Autopilot Mode On"

Name "${NAME}"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\${NAME}"
InstallDirRegKey HKCU "Software\${NAME}" "InstallDir"
Icon "${ICON}"
UninstallIcon "${ICON}"

!include "MUI2.nsh"
!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_FINISHPAGE_RUN "$INSTDIR\autopilot-ui.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Open ${NAME}"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetOutPath "$INSTDIR"
  ; A running copy keeps its file locked. The update flow closes the window
  ; first, so this only matters for a manual re-install.
  File "/oname=autopilot-ui.exe" "${EXE}"
  File "/oname=autopilot-process-guardian.exe" "${GUARDIAN}"
  File "/oname=autopilot-fake-agent.exe" "${PRACTICE}"
  File "/oname=icon.ico" "${ICON}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateDirectory "$SMPROGRAMS\${NAME}"
  CreateShortcut "$SMPROGRAMS\${NAME}\${NAME}.lnk" "$INSTDIR\autopilot-ui.exe" "" "$INSTDIR\icon.ico"

  WriteRegStr HKCU "Software\${NAME}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${KEY}" "DisplayName" "${NAME}"
  WriteRegStr HKCU "${KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${KEY}" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "${KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${KEY}" "NoRepair" 1
SectionEnd

; Tickets and the profile live in the folders the owner chose, never here, so
; removing the program removes no work.
Section "Uninstall"
  Delete "$INSTDIR\autopilot-ui.exe"
  Delete "$INSTDIR\autopilot-process-guardian.exe"
  Delete "$INSTDIR\autopilot-fake-agent.exe"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${NAME}\${NAME}.lnk"
  RMDir "$SMPROGRAMS\${NAME}"
  DeleteRegKey HKCU "${KEY}"
  DeleteRegKey HKCU "Software\${NAME}"
SectionEnd
