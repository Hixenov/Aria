# Music to chat manual check (0.2.3)

Checklist for 0.2.3 (spec `docs/superpowers/specs/2026-11-17-aria-music-to-chat-design.md`,
section 8): the music copied into the second device, the microphone sent there, the microphone
hotkey and icon, and finding the virtual cable. Each line says who verified it:

- **agent**: run by the coding agent on the Windows PC described below, with no sound judged.
- **user**: has to be done by a person with the app in front of them. Audio cannot be judged by an agent.
- **pending**: not verified by anyone yet. Replace `pending` with `user` (and the date) when you have checked it.

Run the release build (`cargo build --release --locked`, then `target\release\aria.exe`). Back up
your Aria data folder first. You need a virtual audio cable (for example VB-CABLE), a microphone,
a voice chat program (Discord, for example) and a second person, or a second account, to listen.

**Nobody has run 0.2.3 by hand and nobody has listened to it.** No test, and no check by the
agent, opened a real microphone: the microphone is tested with a fake capture only. No virtual
cable and no chat program was tried.

## What automated tests cover

These run in `cargo test --workspace --locked`. They are real tests, with fake devices, but none
of them replaces listening.

| Area | What is tested |
|------|----------------|
| Capture | The capture ring holds one second, never blocks, drops what does not fit and counts it; only whole frames go in and out; skipping to the newest frames; a closed or failed stream reports failed; a refused open (and a refusal by the system) is an error. The real microphone test is `#[ignore]`d, for a person only |
| Music copy | A track loaded onto an output that is already open is shared; at the end of the queue the chat stays silent while the next track waits, and both start in step; turning sharing on mid-track leaves the local output bit-for-bit as it was. With sharing on, the second output gets the music at its own volume and the local output is bit-for-bit what it is with sharing off; no copy while off, without a second device, or when the second device is the music's (both by id and as the system default); local mute and volume leave the copy alone; pause and seek act on it; it is resampled to the second output's rate; a second output that is never pulled does not stall the music; a reopen of the local output keeps the second output and the copy carries on; exclusive mode stays "Bit-perfect" with sharing on, and the second output is shared; sounds and the copy share the second output; the second output stays open while music is shared and playing and closes after a pause |
| Microphone | A virtual cable is never opened as the microphone, chosen or as the default; a failed second output fails the microphone, which comes back when the second output opens again; a Windows access-denied error counts as a refusal. It reaches the second output at its volume and never the local output; mono reaches both channels; above 100 % it amplifies and clips; turning it off closes the capture; no second device, or the second device being the music's, opens no microphone; moving the music onto the second device closes it; a whole second captured at once leaves at most 40 ms, the newest; steady speech is not dropped; a capture that fails to open, is refused or is lost reports it, and is retried only at the next change; it is resampled; a change of second device moves it; the copy, the microphone and the sounds fit on one output together |
| Core | Old settings load with sharing and the microphone off; the volumes are clamped (a value that is not a number becomes the default); the engine is told the microphone is off without a second device; the toggle command flips the switch, saves it and gives its notice, with the library off too; the microphone state follows the engine and a refusal gives its own notice; the microphone hotkey is checked against the sound hotkeys, "Stop all" and Aria's own shortcuts, both ways; the input devices are listed on request without opening one; the virtual cable is recognised by name |
| App | The new Settings rows (disabled without a second device, the same-device warning, the switches, the volume bars sending only on release, the microphone list, the hotkey row); the found-cable line and "Use it", the guide when none is found, the chat hint for VB-CABLE and for other cables; the player-bar icon only while on (a click toggles) and the crossed-out icon on a failure; the hotkey fires the toggle, is saved, and is refused in the dialog when a sound has it; the Russian and Armenian locale keys match the English ones |

Not covered by any test: anything audible; a real microphone; a real virtual cable or chat
program; the latency; the Windows microphone mark; how the rows and icons look.

## What the agent ran on real hardware

The agent opened no microphone and played nothing through a cable. PC: Windows 11 Pro for
Workstations.

| Check | Who | Result |
|-------|-----|--------|
| `aria.exe` and `Aria-0.2.3-setup.exe` built from `93ba680` | agent | Yes, 2026-11-17 |
| Silent per-user install of `Aria-0.2.3-setup.exe` | agent | Installed into `%LOCALAPPDATA%\Programs\Aria`; `aria.exe` matched the build byte for byte; the uninstall entry read "Aria 0.2.3"; Start menu shortcuts made |
| Silent uninstall | agent | The program folder, the uninstall entry and the shortcuts were removed; settings and the library were left as they were (hashes compared) |
| The same `aria.exe` started with a real library | user | Started four times on 2026-11-17 without an error in the log, and decoded MP3 tracks. Nothing else about the run was reported |

## What you need to check by hand

### Setting it up

| Check | Who |
|-------|-----|
| With no cable installed, Settings, Soundboard shows the three-step guide; the link opens VB-CABLE's official page in the browser | pending |
| With VB-CABLE installed and no second device chosen, Settings shows "Found a virtual cable: CABLE Input (VB-Audio Virtual Cable)" and "Use it" makes it the second device | pending |
| With the cable chosen, the hint says to choose "CABLE Output" as the microphone in the chat program | pending |
| Choosing your own music device as the second device: the warning shows, and the chat hears nothing from Aria | pending |

### Music in the chat

| Check | Who |
|-------|-----|
| With the cable as the second device and as the chat program's microphone, and "Music also to the second device" on, the other side hears the music | pending |
| "Music volume in the chat" changes how loud the music is for the other side, and not for you | pending |
| Your own mute and volume do not silence or change the music in the chat | pending |
| Pause, seek, next and previous act on what the other side hears too | pending |
| Exclusive mode on with sharing on: your own indicator still shows "Bit-perfect" for a 16 or 24-bit FLAC, and the chat still hears the music | pending |
| Turning sharing on in the middle of a track: no break in your own music; the chat hears a moment of silence, then the music in step | pending |
| The music and your voice reach the other side together, without an echo of your own voice in your headphones | pending |

### The microphone

| Check | Who |
|-------|-----|
| "Send my microphone" on: the other side hears your voice together with the music and the sounds | pending |
| "Microphone volume" at 50 %, 100 % and 200 % | pending |
| The microphone hotkey turns it on and off with a game in the foreground, and with Aria minimized; the "Microphone on" / "Microphone off" notice appears | pending |
| Latency from speaking to the other side hearing it (target under 60 ms from the microphone to the cable; measure, for example, by recording the cable's output while tapping the microphone) | pending |
| The Windows microphone-in-use mark shows only while the switch is on (and a second device is chosen), and goes away when it is turned off | pending |
| The player-bar microphone icon shows only while the microphone is open; its tooltip reads correctly; a click turns it off | pending |
| Unplugging the microphone while it is open: a notice, the crossed-out icon, and music and sounds go on; choosing another microphone or toggling the switch tries again | pending |
| Microphone access denied (Windows Settings, Privacy, Microphone, "Let desktop apps access your microphone" off): the notice says where to allow it | pending |
| Choosing a microphone other than the system default | pending |
| "CABLE Output" chosen as the microphone, or set as the Windows default recording device with "System default": Settings warns, the microphone is not opened, a notice says why, and the chat hears no echo | pending |
| Change the cable's format in the Windows sound settings while the microphone is on: the crossed-out icon appears, and the microphone comes back with the next sound or Play | pending |

### Translations

The Russian and Armenian texts of the new Settings rows, the cable guide, the notices and the
player-bar tooltips: read by a native speaker, and they fit on a narrow window.

| Check | Who |
|-------|-----|
| Russian texts read naturally and fit | pending |
| Armenian texts read naturally and fit | pending |
