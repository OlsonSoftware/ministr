; ministr Tauri NSIS installer hooks.
;
; Tauri's NSIS template (per tauri-bundler v2.0.1-beta.15 / PR #9731)
; !includes this file when bundle.windows.nsis.installerHooks is set, then
; conditionally !insertmacro's each NSIS_HOOK_* macro at the matching point
; in the install/uninstall flow (`!ifmacrodef NSIS_HOOK_FOO` guards mean
; missing macros are simply skipped). LogicLib.nsh is already !included by
; the template so ${If}/${EndIf} work here.
;
; What this file adds:
;
;   1. Stages the bundled `ministr-cli` sidecar at $INSTDIR\bin\ministr.exe
;      so users get a `ministr` command (not `ministr-cli`) on PATH.
;
;   2. Delegates PATH wiring to `ministr setup` via nsExec — the same
;      subcommand install.sh / install.ps1 / `just reinstall` use, backed
;      by the onpath crate. nsExec is built into NSIS; we deliberately
;      avoid the EnVar plugin because Tauri's NSIS toolchain does NOT
;      bundle it (it ships only nsis_tauri_utils.dll, which has no PATH
;      primitives).
;
;   3. Delete-before-copy works around tauri/tauri#15134 (sidecar not
;      always replaced on reinstall).

!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Staging ministr CLI shim..."

  ; Stage the sidecar under bin/ as `ministr.exe`. Tauri's bundler drops
  ; the host-triple-suffixed sidecar into $INSTDIR as `ministr-cli.exe`;
  ; we copy it to bin/ministr.exe so users type `ministr`, not
  ; `ministr-cli`. Delete-before-copy works around tauri/tauri#15134.
  CreateDirectory "$INSTDIR\bin"
  Delete "$INSTDIR\bin\ministr.exe"
  CopyFiles /SILENT "$INSTDIR\ministr-cli.exe" "$INSTDIR\bin\ministr.exe"

  DetailPrint "Adding ministr to PATH via ministr setup..."

  ; Delegate to the staged CLI. `ministr setup` writes HKCU\Environment\PATH
  ; via the onpath crate (Windows path) and broadcasts WM_SETTINGCHANGE so
  ; new shells pick the change up without a logout. Idempotent — if the
  ; bin dir is already on PATH this is a no-op.
  ;
  ; ExecToLog redirects child stdout/stderr into the installer details
  ; pane so failures are visible without crashing the install.
  nsExec::ExecToLog '"$INSTDIR\bin\ministr.exe" setup --bin-dir "$INSTDIR\bin"'
  Pop $0
  ${If} $0 != "0"
    DetailPrint "warning: ministr setup exited with code $0 — PATH may not be wired."
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing ministr from PATH..."

  ; Run `ministr setup --uninstall` while the binary still exists at
  ; $INSTDIR\bin\ministr.exe (Tauri's stock uninstall code wipes $INSTDIR
  ; *after* this hook runs). Symmetric with the POSTINSTALL path.
  ${If} ${FileExists} "$INSTDIR\bin\ministr.exe"
    nsExec::ExecToLog '"$INSTDIR\bin\ministr.exe" setup --bin-dir "$INSTDIR\bin" --uninstall'
    Pop $0
    ; Don't fail the uninstall if PATH cleanup hiccups — better a stale
    ; PATH entry the user can clean up than aborting the rest of the
    ; uninstall sequence.
  ${EndIf}

  Delete "$INSTDIR\bin\ministr.exe"
  RMDir "$INSTDIR\bin"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
