; Kasumi installer hooks (included by Tauri's NSIS template).

; Asks a running Kasumi (or StatusNotch, its old name) to quit through its named
; event and waits up to 5 s for its instance mutex to go away. A graceful exit
; hands the notification area back to Explorer; the template's own check would
; otherwise force-kill it.
!macro AppQuit NAME
  System::Call 'kernel32::OpenEventW(i 0x0002, i 0, w "Local\${NAME}.Quit") p .r0'
  ${If} $0 P<> 0
    System::Call 'kernel32::SetEvent(p r0)'
    System::Call 'kernel32::CloseHandle(p r0)'
    StrCpy $1 0
    ${Do}
      System::Call 'kernel32::OpenMutexW(i 0x00100000, i 0, w "Local\${NAME}.Instance") p .r2'
      ${If} $2 P= 0
        ${Break}
      ${EndIf}
      System::Call 'kernel32::CloseHandle(p r2)'
      Sleep 100
      IntOp $1 $1 + 1
      ${If} $1 >= 50
        ${Break}
      ${EndIf}
    ${Loop}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro AppQuit Kasumi
  !insertmacro AppQuit StatusNotch
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro AppQuit Kasumi
!macroend

; "Delete the application data" also removes the config, presets and log, which
; live in %APPDATA%\Kasumi rather than the bundle-id folder.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$APPDATA\Kasumi"
  ${EndIf}
!macroend
