Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!include "FileFunc.nsh"
!include "MUI2.nsh"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kawoosh"

Name "Kawoosh"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\Kawoosh"

!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Var Fresh

Function .onInit
  ; Keep two setup processes from competing for Kawoosh.new.
  System::Call 'kernel32::CreateMutexW(p 0, i 0, w "Local\KawooshSetup") p .r0 ?e'
  Pop $1
  StrCmp $0 0 busy
  StrCmp $1 183 busy done
  busy:
    MessageBox MB_ICONEXCLAMATION|MB_OK "Kawoosh Setup is already running." /SD IDOK
    SetErrorLevel 1
    Abort
  done:
FunctionEnd

Section "Install"
  SetShellVarContext current
  InitPluginsDir
  ${GetParent} "$INSTDIR" $0
  ClearErrors
  CreateDirectory "$0"
  IfErrors failed
  GetTempFileName $Fresh "$0"
  IfErrors failed
  Delete "$Fresh"
  IfErrors failed
  SetOutPath "$Fresh"
  IfErrors failed
  File /r "${BUNDLE}\*"
  IfErrors failed
  WriteUninstaller "$Fresh\Uninstall.exe"
  IfErrors failed

  SetOutPath "$PLUGINSDIR"
  CopyFiles /SILENT "$Fresh\kawoosh-update.exe" "$PLUGINSDIR"
  IfErrors failed
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/STAGE" $1
  StrCpy $2 "--stage"
  IfErrors 0 +2
  StrCpy $2 "--install"
  ClearErrors
  ExecWait '"$PLUGINSDIR\kawoosh-update.exe" $2 "$INSTDIR" "$Fresh" "${VERSION}"' $1
  IfErrors failed
  StrCmp $1 10 registered
  StrCmp $1 0 registered failed

  registered:
    SetOutPath "$PROFILE"
    CreateShortCut "$SMPROGRAMS\Kawoosh.lnk" "$INSTDIR\kawoosh.exe"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Kawoosh"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "qxuken"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\kawoosh.exe,0"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
    StrCmp $1 10 0 done
    DetailPrint "Update ready. Use :relaunch in the running Kawoosh to install it."
    IfSilent done
    MessageBox MB_ICONINFORMATION|MB_OK "Update ready. Use :relaunch in Kawoosh to install it."
    Goto done

  failed:
    SetOutPath "$TEMP"
    StrCmp $Fresh "" report
    RMDir /r "$Fresh"
  report:
    StrCpy $2 "Could not install Kawoosh. Check that the installation folder is writable and try again."
    FileOpen $3 "$PLUGINSDIR\install-error.txt" r
    IfErrors show
    FileRead $3 $2
    FileClose $3
  show:
    MessageBox MB_ICONSTOP|MB_OK "$2" /SD IDOK
    SetErrorLevel 1
    Abort
  done:
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  IfFileExists "$INSTDIR\kawoosh.exe" 0 remove
  ClearErrors
  CopyFiles /SILENT "$INSTDIR\kawoosh-update.exe" "$PLUGINSDIR"
  IfErrors busy
  ExecWait '"$PLUGINSDIR\kawoosh-update.exe" --running "$INSTDIR"' $0
  IfErrors busy
  StrCmp $0 0 remove
  busy:
    MessageBox MB_ICONEXCLAMATION|MB_OK "Quit Kawoosh before uninstalling it." /SD IDOK
    SetErrorLevel 1
    Abort

  remove:
    Delete "$INSTDIR\kawoosh.exe"
    Delete "$INSTDIR\kawoosh.com"
    Delete "$INSTDIR\kawoosh-edit.exe"
    Delete "$INSTDIR\kawoosh-update.exe"
    Delete "$INSTDIR\conpty.dll"
    Delete "$INSTDIR\OpenConsole.exe"
    Delete "$INSTDIR\kawoosh.lib"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir /r "$INSTDIR\fonts"
    RMDir /r "$INSTDIR\include"
    RMDir "$INSTDIR"
    IfFileExists "$INSTDIR.new\ready" 0 shortcuts
    IfFileExists "$INSTDIR.new\kawoosh-update.exe" 0 shortcuts
    RMDir /r "$INSTDIR.new"
  shortcuts:
    Delete "$SMPROGRAMS\Kawoosh.lnk"
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
SectionEnd
