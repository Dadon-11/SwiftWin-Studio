; SwiftWin installer hooks (tauri.conf.json > bundle > windows > nsis > installerHooks).
;
; `swiftwin mcp`, which Claude Code starts for every session once it is registered, keeps
; bin\swiftwin.exe open. The installer could then not replace it, and a silent update quietly
; kept the old command line (seen on the owner's PC). Windows lets a running .exe be renamed,
; so it is moved aside first; the running servers keep using the old copy, new ones start the
; new file. Old copies no longer in use are deleted on the next update and on uninstall.

!macro NSIS_HOOK_PREINSTALL
  Delete "$INSTDIR\bin\swiftwin.old*.exe"
  IfFileExists "$INSTDIR\bin\swiftwin.exe" 0 swiftwin_cli_free
    Push $R9
    System::Call "kernel32::GetTickCount() i .R9"
    Rename "$INSTDIR\bin\swiftwin.exe" "$INSTDIR\bin\swiftwin.old-$R9.exe"
    Pop $R9
  swiftwin_cli_free:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\bin\swiftwin.old*.exe"
  RMDir "$INSTDIR\bin"
!macroend
