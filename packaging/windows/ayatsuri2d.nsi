; Ayatsuri2D Windows installer (NSIS). Built on Linux or Windows by
; packaging/windows/package.sh, which passes the defines below:
;   VERSION   0.5.1        NUMVER  0.5.1.0
;   SRC       folder holding ayatsuri.exe, ayatsuri-launcher.exe and docs
;   OUTFILE   where the setup .exe goes
Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel admin
ManifestDPIAware true

!include "MUI2.nsh"
!include "x64.nsh"
!include "FileFunc.nsh"

!define APP      "Ayatsuri2D"
!define PUBLISHER "Yoru_KenomoVT"
!define URL      "https://github.com/Yoru-KenomoVT/ayatsuri_2d"
!define UNINST   "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"

Name "${APP} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${APP}"
InstallDirRegKey HKLM "Software\${APP}" "InstallDir"
BrandingText "${APP} ${VERSION}"

VIProductVersion "${NUMVER}"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "${PUBLISHER}, GPL-3.0"
VIAddVersionKey "FileDescription" "${APP} installer"

!define MUI_ICON "${SRC}\ayatsuri2d.ico"
!define MUI_UNICON "${SRC}\ayatsuri2d.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\ayatsuri-launcher.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APP}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${SRC}\LICENSE.txt"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Japanese"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "${APP} needs 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Section "${APP}" SecApp
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${SRC}\ayatsuri.exe"
  File "${SRC}\ayatsuri-launcher.exe"
  File "${SRC}\ayatsuri2d.ico"
  File "${SRC}\LICENSE.txt"
  File "${SRC}\LICENSING.md"
  File "${SRC}\README.md"
  File "${SRC}\credits.tsv"

  CreateDirectory "$SMPROGRAMS\${APP}"
  CreateShortcut "$SMPROGRAMS\${APP}\${APP}.lnk" "$INSTDIR\ayatsuri-launcher.exe" "" "$INSTDIR\ayatsuri2d.ico"
  CreateShortcut "$SMPROGRAMS\${APP}\Uninstall ${APP}.lnk" "$INSTDIR\uninstall.exe"

  WriteRegStr HKLM "Software\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "${UNINST}" "DisplayName" "${APP}"
  WriteRegStr HKLM "${UNINST}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${UNINST}" "URLInfoAbout" "${URL}"
  WriteRegStr HKLM "${UNINST}" "DisplayIcon" "$INSTDIR\ayatsuri2d.ico"
  WriteRegStr HKLM "${UNINST}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKLM "${UNINST}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST}" "NoRepair" 1
  ; size in KB for Add/Remove Programs
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKLM "${UNINST}" "EstimatedSize" "$0"
  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\ayatsuri-launcher.exe" "" "$INSTDIR\ayatsuri2d.ico"
SectionEnd

Section "Open .aom files with ${APP}" SecAssoc
  WriteRegStr HKLM "Software\Classes\.aom" "" "Ayatsuri2D.Model"
  WriteRegStr HKLM "Software\Classes\Ayatsuri2D.Model" "" "Ayatsuri2D model"
  WriteRegStr HKLM "Software\Classes\Ayatsuri2D.Model\DefaultIcon" "" "$INSTDIR\ayatsuri2d.ico"
  WriteRegStr HKLM "Software\Classes\Ayatsuri2D.Model\shell\open\command" "" '"$INSTDIR\ayatsuri-launcher.exe" "%1"'
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

LangString DESC_App ${LANG_ENGLISH} "The editor and its launcher."
LangString DESC_Desktop ${LANG_ENGLISH} "A shortcut on the desktop."
LangString DESC_Assoc ${LANG_ENGLISH} "Double-click .aom models to open them."
LangString DESC_App ${LANG_JAPANESE} "エディターとランチャー。"
LangString DESC_Desktop ${LANG_JAPANESE} "デスクトップのショートカット。"
LangString DESC_Assoc ${LANG_JAPANESE} ".aom モデルをダブルクリックで開く。"
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} $(DESC_App)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} $(DESC_Desktop)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAssoc} $(DESC_Assoc)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  SetRegView 64
  Delete "$INSTDIR\ayatsuri.exe"
  Delete "$INSTDIR\ayatsuri-launcher.exe"
  Delete "$INSTDIR\ayatsuri2d.ico"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\LICENSING.md"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\credits.tsv"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}\${APP}.lnk"
  Delete "$SMPROGRAMS\${APP}\Uninstall ${APP}.lnk"
  RMDir "$SMPROGRAMS\${APP}"
  Delete "$DESKTOP\${APP}.lnk"
  ; only drop the .aom association if it is still ours
  ReadRegStr $0 HKLM "Software\Classes\.aom" ""
  StrCmp $0 "Ayatsuri2D.Model" 0 +2
    DeleteRegKey HKLM "Software\Classes\.aom"
  DeleteRegKey HKLM "Software\Classes\Ayatsuri2D.Model"
  DeleteRegKey HKLM "${UNINST}"
  DeleteRegKey HKLM "Software\${APP}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd
