; ministr Tauri NSIS installer hooks.
;
; Tauri's NSIS template (per tauri-bundler v2.0.1-beta.15 / PR #9731)
; !includes this file when bundle.windows.nsis.installer_hooks is set, then
; !insertmacro's each of the four NSIS_HOOK_* macros at the matching point
; in the install/uninstall flow. All four must be defined (empty stubs are
; fine) — Tauri inserts them unconditionally.
;
; What this file adds on top of the default Tauri NSIS install:
;
;   1. Stages the bundled `ministr-cli` sidecar at $INSTDIR\bin\ministr.exe
;      so users get a `ministr` command (not `ministr-cli`) on PATH.
;
;   2. Adds $INSTDIR\bin to the per-user PATH (HKCU\Environment) via the
;      EnVar plugin, which is bundled with Tauri's NSIS toolchain and
;      handles dedupe + WM_SETTINGCHANGE broadcast for us.
;
;   3. On uninstall, removes the PATH entry and the bin dir before Tauri's
;      stock uninstaller wipes $INSTDIR.
;
; The reinstall caveat (tauri/tauri#15134) — the NSIS installer doesn't
; always replace externalBin sidecars on reinstall over an existing version.
; We work around it by deleting our renamed copy before re-copying so the
; new $INSTDIR\ministr-cli.exe is what users get.

!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing ministr CLI shim and updating PATH..."

  ; Stage the sidecar under bin/ as `ministr.exe`. Tauri's bundler drops
  ; the host-triple-suffixed sidecar into $INSTDIR as `ministr-cli.exe`;
  ; we copy it to bin/ministr.exe so users type `ministr`, not
  ; `ministr-cli`. Delete-before-copy works around tauri/tauri#15134
  ; (sidecar not replaced on reinstall).
  CreateDirectory "$INSTDIR\bin"
  Delete "$INSTDIR\bin\ministr.exe"
  CopyFiles /SILENT "$INSTDIR\ministr-cli.exe" "$INSTDIR\bin\ministr.exe"

  ; Per-user PATH update via EnVar plugin (bundled with Tauri NSIS).
  ; SetHKCU pins scope to HKCU\Environment — matches our currentUser
  ; install mode. AddValue is idempotent: if the entry is already
  ; present it's a no-op, no duplicate.
  EnVar::SetHKCU
  EnVar::AddValue "PATH" "$INSTDIR\bin"
  Pop $0
  ${If} $0 != "0"
    DetailPrint "warning: failed to add $INSTDIR\bin to PATH (EnVar code $0). Add it manually if `ministr` doesn't resolve."
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing ministr from PATH..."

  ; Mirror the install: scope HKCU, drop our bin entry, then tear down
  ; the bin dir so Tauri's stock uninstall finds a clean $INSTDIR.
  EnVar::SetHKCU
  EnVar::DeleteValue "PATH" "$INSTDIR\bin"
  Pop $0
  ; Don't fail uninstall if PATH cleanup hiccups — better to leave a
  ; stale PATH entry the user can clean up than to abort the rest of
  ; the uninstall.

  Delete "$INSTDIR\bin\ministr.exe"
  RMDir "$INSTDIR\bin"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
