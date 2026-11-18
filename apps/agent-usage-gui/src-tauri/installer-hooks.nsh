; Preserve the preference across upgrades, but remove our PATH entry on uninstall.
; These maintenance modes exit before starting Tauri or the single-instance plugin.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    ExecWait '"$INSTDIR\agent-usage-gui.exe" --unregister-cli' $0
    ${If} $0 != 0
      MessageBox MB_OK|MB_ICONSTOP "Could not remove the Agent Usage CLI PATH entry. Uninstall was stopped; please disable the CLI in Settings and try again."
      Abort
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ExecWait '"$INSTDIR\agent-usage-gui.exe" --restore-cli' $0
  ${If} $0 != 0
    DetailPrint "CLI registration could not be restored. Open Settings to check for an existing CLI installation."
  ${EndIf}
!macroend
