; Uninstall hooks for the NSIS package.
;
; Tauri's own installer template already removes the program, the shortcuts, the Add/Remove
; entry and the `Run` value that "start with Windows" writes. Three things it cannot know
; about are this application's, and they are here.
;
; 1. **The status-line wrapper.** If the user asked for it from the settings page, Claude
;    Code's `settings.json` now points at `$INSTDIR\nazar-statusline.exe`, and the next line
;    of this uninstaller deletes that file. A status line whose command does not exist is a
;    broken prompt in another program, caused by uninstalling ours. So the wrapper is asked
;    to undo its own installation first, while it still exists: it restores the exact
;    `statusLine` object it replaced, or removes the key if there was none, and prints
;    "Nothing to do" when it was never installed. It refuses rather than guesses when its
;    record and its backup disagree, and a refusal here is not fatal — the uninstall carries
;    on and `docs/statusline-wrapper.md` documents the by-hand restore.
;
; 2. **The `StartupApproved` residue.** Windows keeps a second record of every startup
;    entry, next to the `Run` value, and it is the one Task Manager's Startup tab reads.
;    `tauri-plugin-autostart` never writes it — Windows does, the first time the user
;    touches the entry there — and nothing removes it. It is inert on its own, but it is a
;    row with this application's name in a list a person reads, left behind by an uninstall
;    that claims to leave nothing. WP5 found it by looking; this is the removal.
;
; 3. **`HKCU\Software\qarpus\nazar-tray`.** One value, the directory the last install went
;    to, so that a reinstall offers the same one. Tauri's template writes it always and
;    removes it only when the user ticks "delete application data" — so a silent uninstall,
;    which is what `winget uninstall` performs, leaves a key behind pointing at a directory
;    that no longer exists. Measured, not assumed: it was the one thing still on the machine
;    after the WP7 acceptance run. There is no previous install to remember once the
;    uninstall has finished, so it goes with it, and the parent goes too if it is empty.
;
; 4. **`%APPDATA%\nazar`.** The settings and the notification history. Removed only when the
;    user ticks "delete application data" on the uninstall page, because it is the answer to
;    a question the installer asks. A silent uninstall never sees that page, so it keeps the
;    folder; reinstalling then finds the settings where they were. Unlike the registry key
;    above, this is data a person chose, not a note the installer left itself.
;    `docs/RELEASE.md` says how to remove it by hand.
;
; **`~/.nazar` is never touched, by any path through this file.** It holds `limits.json`,
; which is not ours alone: Nazar reads it, and a user may be running Nazar without ever
; having had this tray installed. It also holds the status-line captures the wrapper writes
; and the record of what the wrapper replaced, which is the only machine-readable copy of
; the status line the user had before. Deleting any of that on uninstall would be this
; program removing another program's input.

!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_POSTINSTALL
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} ${FileExists} "$INSTDIR\nazar-statusline.exe"
    nsExec::ExecToLog '"$INSTDIR\nazar-statusline.exe" uninstall'
    Pop $0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU \
      "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${PRODUCTNAME}"

    DeleteRegKey HKCU "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"

    ${If} $DeleteAppDataCheckboxState = 1
      RMDir /r "$APPDATA\nazar"
    ${EndIf}
  ${EndIf}
!macroend
