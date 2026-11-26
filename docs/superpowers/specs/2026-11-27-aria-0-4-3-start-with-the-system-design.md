# Aria 0.4.3: start with the system

Status: approved. The user was offered "Quietly in the tray" (my pick), "Minimized" or "A normal
window", and answered "иду спать доделай" ("going to sleep, finish it"). That leaves the choice to
me, so it is my pick.

## Intent

This is Phase 4's third piece, after media keys (0.4.0) and the tray (0.4.2). With one switch,
Aria starts when the user logs in and waits quietly in the tray, ready for a media key or a
click.

## Goals

- **The switch.** Settings, General gets "Start with the system" (`start_with_system`), off by
  default.
  - When on, Aria makes the system start it at login with the argument `--autostart`.
  - When off, Aria removes that entry.
- **The entry per system** (always for the current user, never for the whole machine):
  - **Windows.** The value `Aria` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`,
    set to `"<path to aria.exe>" --autostart`. This is the list Task Manager's Startup tab
    shows; disabling Aria there is respected, because Windows keeps that apart, in its own
    place.
  - **macOS.** `~/Library/LaunchAgents/io.github.hixenov.aria.plist`, with `RunAtLoad`, and
    `ProgramArguments` set to the executable and `--autostart`.
  - **Linux and the BSDs.** `$XDG_CONFIG_HOME/autostart/io.github.hixenov.aria.desktop`
    (`~/.config` when `XDG_CONFIG_HOME` is unset). Its `Exec` line is the executable, quoted as
    the desktop entry spec says, with `--autostart`.
- **The entry is checked at every start.**
  - With the switch on, the entry is (re)written when it is missing or names another path, so
    a portable Aria that moved keeps working.
  - With the switch off, an entry is removed only if it is Aria's own: the right name, and
    `--autostart` in its command.
  - Nothing else is ever touched.
- **What a login start looks like.** Aria starts with its window hidden and its icon in the
  tray. Nothing plays by itself.
  - If Aria can't hide there, its window appears minimized instead. That covers no tray icon,
    Wayland, and Linux or the BSDs without the single-instance claim (the same `can_hide()`
    rule as 0.4.2).
  - Aria is never left running out of reach.
- **A login start while Aria already runs** quits quietly. It doesn't show the running window,
  and it doesn't show the "already running" message.
- **`--autostart` is never taken for a file to open.**
- **Failures.** If the entry can't be written or removed, Aria shows a notice ("Couldn't change
  whether Aria starts with the system."). The switch keeps what the user chose, and Aria tries
  again at the next start.
- **The Windows uninstaller** deletes the `Run` value `Aria` of the user who uninstalls.
  (Inno: `ValueType: none` with `uninsdeletevalue`.)

## Non-goals

- No checkbox in the installer: the switch in Aria is the one place.
- No entry for all users. No "start playing at login".
- No change to the tray switches. A login start always waits in the tray, whatever "Minimize
  to tray" says.

## Constraints

- No `unwrap` or `expect` outside tests, no `unsafe`, no `#[allow]`.
- Tests never touch the real `Run` key, LaunchAgents or autostart folder.
  - The file entries are tested in temp folders.
  - The registry code is tested under a throwaway `HKCU\Software\AriaTest-<unique>` key,
    deleted afterwards.
  - The app's logic is tested with a fake.
- Windows registry access uses `winreg` 0.56 (MIT; already in Cargo.lock, through
  `embed-resource`), as a Windows-only dependency. Nothing else is new.

## Approaches considered

1. **Aria writes the entry itself, per system (chosen).** It needs one small dependency, on
   Windows only, and stays fully under Aria's control and tests.
2. The `auto-launch` crate. It brings its own choices: AppleScript login items on macOS, which
   needs an Automation permission prompt, and its own naming. It has more dependencies, and it
   would still need the ownership checks.
3. The Windows installer's Startup-folder shortcut, as an install-time task. It can't follow the
   in-app switch, and it does nothing on macOS or Linux.

## My calls

- The name is `--autostart`, checked anywhere in the arguments.
- The hidden start is `ViewportBuilder::with_visible(false)`, with `hidden = true` in the app
  from the first frame. eframe 0.37 runs `logic` for invisible windows, which the tray already
  relies on.
- The quiet second copy is a new `instance::claim_quietly`, which never reaches the running
  copy.
- Version 0.4.3.
