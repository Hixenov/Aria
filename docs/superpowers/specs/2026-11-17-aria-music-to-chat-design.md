# Aria — Music to Chat (0.2.3) Design

- **Date:** 2026-11-17
- **Status:** Design approved in conversation; pending written-spec review
- **Release:** 0.2.3
- **Builds on:** Phase 2c (0.2.2), `2026-11-15-aria-phase2c-lossless-soundboard-design.md`: the
  second output device and the soundboard.

## 1. Context and intent

In 0.2.2 the soundboard can play sounds into a second output device, such as a virtual audio cable
that a voice-chat program uses as its microphone. The chat then hears the sounds but not the
listener's voice, and never the music.

The user wants friends in a voice chat to hear the music Aria plays, together with the user's own
voice, in the way a music bot works in a chat. So 0.2.3 sends two more things into the second
device, next to the sounds:

1. a copy of the music that is playing;
2. the user's microphone.

In the chat program, the user picks the virtual cable as the microphone. The chat then hears
voice, music and sounds as one stream.

Everything stays local. Aria sends audio only to the devices the user picked. It never records,
stores or sends audio anywhere else.

### Decisions made during design

| Decision | Choice |
|----------|--------|
| Release | 0.2.2 first, then this as 0.2.3 |
| Microphone mode | An on/off switch, plus a global hotkey to toggle it. Push-to-talk is left to the chat program |
| Music in the cable | A switch, off by default; the copy has its own volume, default 50 % |
| Listening to yourself | Not in 0.2.3 |
| Voice processing (noise suppression, equaliser, compressor) | Not in 0.2.3; left to the chat program |
| Platforms | Built for all three; checked by hand on Windows only |

## 2. Scope

### In 0.2.3

- **"Share music":** the music that plays locally is also mixed into the second device.
- **"Send my microphone":** a chosen input device is captured and mixed into the second device.
- Separate volumes for the shared music and for the microphone.
- A global hotkey that turns the microphone on or off.
- A microphone indicator in the player bar while the microphone is open.
- Settings and notices in English, Russian and Armenian.
- `docs/PRIVACY.md` updated: Aria now reads the microphone when, and only when, the user turns it on.

### Not in 0.2.3

- Hearing yourself (monitoring); push-to-talk inside Aria; voice processing of any kind.
- Recording the microphone or the mix to a file.
- Installing or bundling a virtual cable.
- Sending to more than one second device.
- Sharing music when no second device is chosen.

## 3. Behaviour

### 3.1 Share music

- A switch in Settings, "Sound panel": **"Music also to the second device"**. It is off by default
  and can only be turned on while a second device is chosen. Without a second device the switch is
  disabled, with a line saying a second device is needed.
- While the switch is on, every audio frame the music sends to the listener's output is also sent
  to the second device, at that device's format. The engine converts the format as it does for
  sounds.
- **Volume.**
  - A slider, "Music volume in the chat", runs from 0 to 100 % and defaults to 50 %.
  - It applies only to the copy in the cable.
  - The player's volume and mute apply only to the listener's own output. Muting locally does not
    silence the chat, so the user can turn their own speakers down and keep the music going for
    others.
- **Following the music.** Pause, resume, seek, next and previous act on both copies, because they
  are one stream. The 5 ms fades apply to both.
- **Exclusive mode** stays allowed on the listener's own device. The copy for the cable goes
  through a shared output and is resampled if needed. The player-bar indicator still describes only
  the listener's path; sharing does not change that path, so local playback can stay bit-perfect.
- **Reopening.** When the listener's output closes or reopens (a rate change in exclusive mode, a
  device change, a failure), the cable copy pauses for the same moment and carries on with the next
  audio. There is no separate silence beyond the one the listener hears.
- **Timing.** The two copies may drift by up to the difference between the two devices' buffers,
  typically tens of milliseconds. This is not corrected.

### 3.2 Microphone

- **Settings, "Sound panel".**
  - **"Microphone":** a list of input devices, with "System default" first.
  - **"Send my microphone to the second device":** a switch, off by default.
  - **"Microphone volume":** 0 to 200 %, default 100 %. Values above 100 % amplify, and the result
    is clipped at full scale.
  - **"Hotkey to turn the microphone on/off":** set with the existing hotkey dialog and checked by
    the existing rules (spec 2c section 4.6).
- **While the switch is on and a second device is chosen,** Aria opens the chosen input device in
  shared mode and mixes what it captures into the second device continuously.
  - The microphone is never sent to the listener's own device.
  - Opening the microphone does not depend on music or sounds playing.
- **While the switch is off, or no second device is chosen,** no input device is open. Turning the
  switch off closes the input device at once.
- **The hotkey** flips the switch and shows a short notice: "Microphone on" or "Microphone off". It
  works like every other global hotkey, including while Aria is minimized.
- **Indicator.**
  - While the input device is open, the player bar shows a microphone icon.
  - Clicking the icon turns the microphone off, and its tooltip says where the voice is going.
  - When the input device is closed, there is no icon.
- **Format.** The capture is converted to the second device's rate and channels. Mono is sent to
  both channels.
- **Latency target.** Microphone to cable in under 60 ms on this PC, measured by hand (section 8).
  The capture and output buffers are kept small enough for that; the plan sets the sizes.

### 3.3 Where the second device's audio comes from

The second device's output carries up to three kinds of input, each with its own gain:

| Input | Gain | Present when |
|-------|------|--------------|
| Sounds | per sound | as in 0.2.2 |
| Music copy | "Music volume in the chat" | share music is on and the music is playing |
| Microphone | "Microphone volume" | the microphone is on |

- The second output opens when any of these needs it and closes 15 s after the last one stops. An
  open microphone counts as always needing it.
- The sound limit stays 6 voices.
- The mixer's input limit (`MAX_INPUTS`) must hold the music copy, the microphone and the sounds
  together. The plan checks this and raises the limit if needed.
- If the second device is the same as the listener's device, it is used as one output. Then:
  - the music is not copied a second time;
  - the microphone is still not mixed in, so the user never hears their own voice by accident.
  - Settings shows a line warning that this setup does nothing useful for a chat.

## 4. Architecture

- **`aria-audio`.**
  - A capture source behind a trait, with a CPAL implementation and a fake for tests. It opens an
    input device in shared mode and writes captured frames into a lock-free ring. Its callback does
    no allocation, takes no lock and makes no log call.
  - The engine thread pulls from that ring, converts the format and writes into a microphone input
    on the second output.
  - The music copy is a second write of the frames the engine already produces for the listener's
    output. It is resampled to the second output's format and written into a music-copy input there.
  - The engine gains these commands: `SetShareMusic { on, gain }`, `SetMicrophone { device, on, gain }`.
  - It gains these events: `MicrophoneOpened`, `MicrophoneClosed`, `MicrophoneFailed`.
- **`aria-core`.**
  - Settings: `share_music: bool` (false), `share_music_volume: f32` (0.5), `mic_enabled: bool`
    (false), `mic_device: Option<DeviceId>` (none, meaning the system default),
    `mic_volume: f32` (1.0), `mic_toggle_hotkey: Option<Hotkey>` (none). All are lenient serde
    fields, as in 2c.
  - An input-device list next to the output list.
  - A `ToggleMicrophone` command.
  - `PlayerView` gains `microphone: MicState { Off, On, Failed }`.
  - The hotkey uniqueness rules cover the new hotkey together with the sound hotkeys and "Stop all".
- **`aria-app`.** The new settings rows, the microphone icon in the player bar, the hotkey wired
  through the existing hotkey manager, and the new notices.

Nothing blocks the UI thread. No new dependency is expected: CPAL already supports input streams.

## 5. Data

- There is no database change in 0.2.3, and the settings schema version stays the same.
- A 0.2.2 settings file loads with every new field at its default.

## 6. Error handling

- **No input device, or it cannot be opened.**
  - A notice appears: "The microphone could not be opened".
  - The switch stays on and the state is `Failed`.
  - The next change of device or of the switch tries again.
  - Music and sounds are unaffected.
- **The input device disappears while open.** It is closed, with the same notice and state.
- **Microphone access denied by the operating system** (Windows privacy settings, macOS
  permission): a notice tells the user to allow microphone access for desktop apps. The state is
  `Failed`.
- **The second device fails.** As in 0.2.2: a notice, and the music copy and the microphone stop
  until the next attempt. The listener's own playback goes on.
- **Paths in logs** stay redacted. Device names may be logged.

## 7. Privacy

This is the first Aria feature that reads the microphone. The rules:

- The microphone is off by default and is opened only while "Send my microphone" is on and a second
  device is chosen.
- What is captured goes only into the second device's mixer. It is never written to disk, never
  logged, and never sent over a network.
- While it is open, the player bar shows the microphone icon. Windows also shows its own
  microphone-in-use mark.
- `docs/PRIVACY.md` and the README say all of this, and remove the 0.2.2 statement that Aria never
  reads the microphone.

## 8. Testing

- **Engine, with fake outputs and a fake capture source.**
  - With share music on, the second output receives the music at the chat volume, and the local
    output is unchanged.
  - With it off, the second output receives no music.
  - Pause and seek act on both copies.
  - Local mute or volume does not change the copy.
  - The copy is resampled to the second output's rate.
  - A reopen of the local output pauses the copy and carries on.
  - Exclusive mode: the local path is still bit-perfect when nothing else is mixed into it, and the
    copy goes through the second output.
  - The microphone is mixed into the second output at its gain, never into the local output.
  - A mono microphone reaches both channels.
  - Gain above 1.0 amplifies and clips.
  - Turning the microphone off closes the capture.
  - A failed capture gives `MicrophoneFailed`, and music and sounds go on.
  - The second output stays open while the microphone is on, and closes 15 s after everything stops.
  - When the second device is the local device, there is no copy and no microphone.
  - The input limit holds the copy, the microphone and 6 sounds together.
- **Core.**
  - Defaults and lenient loading of the new settings.
  - `ToggleMicrophone` and the hotkey.
  - The uniqueness rule across sound hotkeys, "Stop all" and the microphone hotkey.
  - The `PlayerView` microphone state.
  - The notices.
- **App** (in the test harness):
  - the settings rows, including the switches disabled without a second device;
  - the icon shown only while the microphone is open, and a click on it turning the microphone off;
  - the hotkey row.
- **By hand,** in `docs/testing/music-to-chat-manual-check.md`, with every line starting as pending:
  - with a virtual cable as the second device and the cable as the chat program's microphone,
    friends hear the music and the user's voice together;
  - the chat volume works;
  - local mute does not silence the chat;
  - the microphone hotkey works with a game in the foreground;
  - microphone latency is measured;
  - Windows shows the microphone-in-use mark only while the switch is on;
  - unplugging the microphone;
  - a denied microphone permission;
  - the Russian and Armenian texts.

## 9. Known limitations

- A virtual cable must be installed separately. In the chat program the cable, not the real
  microphone, must be chosen as input.
- The user does not hear their own voice. There is no noise suppression or other voice
  processing in Aria; the chat program's own processing applies on top.
- The music copy and the local music may drift by tens of milliseconds.
- When the local output reopens, for example at a rate change in exclusive mode, the chat hears the
  same short silence.
- Choosing the listener's own device as the second device disables the music copy and the
  microphone.
- Checked by hand on Windows only. On macOS, microphone permission must be granted to Aria.
- The Russian and Armenian texts are not reviewed by a native speaker.

## 10. Definition of done

- Every item in section 2 works.
- The tests in section 8 pass, with `cargo fmt`, `clippy -D warnings` and `cargo deny` clean on
  Windows, Linux and macOS in CI.
- A 0.2.2 settings file loads in 0.2.3 with nothing lost.
- README, CHANGELOG, `docs/PRIVACY.md` and the manual check list are up to date, and say plainly what
  nobody has verified.
- `aria.exe` and `Aria-0.2.3-setup.exe` are built and tested locally (launch; install, launch,
  uninstall), with the PC's real Aria data copied aside first and restored afterwards.
- Merging, tagging and publishing wait for the user's explicit yes.
