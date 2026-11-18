//! The second device as a chat's microphone: the music copied into it, the microphone sent there,
//! and finding the virtual cable that a chat program listens to.
//!
//! Aria never installs or changes a driver. It only recognises a virtual cable that is already
//! installed, by its name, and helps the user pick it.

use aria_audio::{DeviceId, DeviceInfo, EngineCommand, EngineEvent};

pub use aria_audio::is_virtual_cable;

use crate::{
    notice::{Notice, NoticeLevel},
    settings::Settings,
};

/// The microphone could not be opened, or was lost.
pub(crate) const MICROPHONE_FAILED: Notice = Notice::new(NoticeLevel::Warning, "microphone-failed");
/// The operating system refused the microphone.
pub(crate) const MICROPHONE_DENIED: Notice = Notice::new(NoticeLevel::Warning, "microphone-denied");
/// The microphone chosen is a virtual cable, so it was not opened.
pub(crate) const MICROPHONE_IS_CABLE: Notice =
    Notice::new(NoticeLevel::Warning, "microphone-is-cable");
/// The microphone hotkey turned the microphone on.
pub(crate) const MICROPHONE_ON: Notice = Notice::new(NoticeLevel::Info, "microphone-on");
/// The microphone hotkey could not turn the microphone on: there is no second device.
pub(crate) const MICROPHONE_NEEDS_DEVICE: Notice =
    Notice::new(NoticeLevel::Warning, "microphone-needs-device");
/// The microphone hotkey turned the microphone off.
pub(crate) const MICROPHONE_OFF: Notice = Notice::new(NoticeLevel::Info, "microphone-off");

/// The official page of a free virtual cable, offered when none is installed. Aria makes no
/// request to it; the link only opens the user's browser.
pub const CABLE_GUIDE_URL: &str = "https://vb-audio.com/Cable/";

/// Parts of a device name that mark VB-CABLE, whose recording side is "CABLE Output".
const VB_CABLE_MARKS: [&str; 2] = ["cable input", "vb-audio"];

/// Whether the microphone is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MicState {
    /// Closed: turned off, or there is no second device.
    #[default]
    Off,
    /// Open, and going to the second device.
    On,
    /// Turned on, but it could not be opened or was lost.
    Failed,
}

/// Which recording device a chat program must pick to hear the cable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CableSide {
    /// VB-CABLE: "CABLE Output".
    VbCable,
    /// Another cable, whose recording side Aria does not know by name.
    Other,
}

/// The first virtual cable among `devices`.
pub fn find_virtual_cable(devices: &[DeviceInfo]) -> Option<&DeviceInfo> {
    devices.iter().find(|device| is_virtual_cable(&device.name))
}

/// The recording side of the cable named `name`.
pub fn cable_side(name: &str) -> CableSide {
    let name = name.to_lowercase();
    if VB_CABLE_MARKS.iter().any(|mark| name.contains(mark)) {
        CableSide::VbCable
    } else {
        CableSide::Other
    }
}

/// What the engine was last told about the music copy and the microphone, and the microphone's
/// state as the engine reports it.
#[derive(Debug, Default)]
pub(crate) struct Chat {
    share: Option<(bool, f32)>,
    mic: Option<(bool, Option<DeviceId>, f32)>,
    state: MicState,
    /// The open microphone's name, when it has one.
    name: Option<String>,
}

impl Chat {
    /// Takes in the settings. Returns the commands that tell the engine what changed, and whether
    /// the microphone's state changed with them.
    pub(crate) fn on_settings(&mut self, settings: &Settings) -> (Vec<EngineCommand>, bool) {
        let second = settings.soundboard_device.is_some();
        let mut commands = Vec::new();
        let share = (settings.share_music && second, settings.share_music_volume);
        if self.share != Some(share) {
            self.share = Some(share);
            commands.push(EngineCommand::SetShareMusic {
                on: share.0,
                gain: share.1,
            });
        }
        let mic = (
            settings.mic_enabled && second,
            settings.mic_device.clone(),
            settings.mic_volume,
        );
        let mut changed = false;
        if self.mic.as_ref() != Some(&mic) {
            // Off is off at once: the engine closes it, and a failure no longer matters.
            if !mic.0 && self.state != MicState::Off {
                self.state = MicState::Off;
                self.name = None;
                changed = true;
            }
            commands.push(EngineCommand::SetMicrophone {
                on: mic.0,
                device: mic.1.clone(),
                gain: mic.2,
            });
            self.mic = Some(mic);
        }
        (commands, changed)
    }

    /// Acts on what the engine says of the microphone. Returns whether the state changed, and a
    /// notice to show. Other events are not for this.
    pub(crate) fn on_engine(&mut self, event: EngineEvent) -> (bool, Option<Notice>) {
        let (state, notice) = match event {
            EngineEvent::MicrophoneOpened { name } => {
                self.name = name;
                (MicState::On, None)
            }
            EngineEvent::MicrophoneClosed => (MicState::Off, None),
            EngineEvent::MicrophoneIsCable => (MicState::Failed, Some(MICROPHONE_IS_CABLE)),
            EngineEvent::MicrophoneFailed { denied } => (
                MicState::Failed,
                Some(if denied {
                    MICROPHONE_DENIED
                } else {
                    MICROPHONE_FAILED
                }),
            ),
            _ => return (false, None),
        };
        // A close that comes after the switch went off is old news.
        let wanted = self.mic.as_ref().is_some_and(|mic| mic.0);
        let state = if wanted { state } else { MicState::Off };
        let notice = notice.filter(|_| wanted);
        let changed = state != self.state;
        self.state = state;
        (changed, notice)
    }

    pub(crate) fn state(&self) -> MicState {
        self.state
    }

    /// The open microphone's name, when it is open and has one.
    pub(crate) fn name(&self) -> Option<&str> {
        self.name.as_deref().filter(|_| self.state == MicState::On)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened() -> EngineEvent {
        EngineEvent::MicrophoneOpened { name: None }
    }

    #[test]
    fn the_open_microphone_is_named_until_it_closes() {
        let mut chat = Chat::default();
        let on = Settings {
            mic_enabled: true,
            soundboard_device: Some(DeviceId("cable".into())),
            ..Settings::default()
        };
        chat.on_settings(&on);
        assert_eq!(chat.name(), None);
        let usb = EngineEvent::MicrophoneOpened {
            name: Some("USB Microphone".into()),
        };
        assert_eq!(chat.on_engine(usb), (true, None));
        assert_eq!(chat.name(), Some("USB Microphone"));
        chat.on_engine(EngineEvent::MicrophoneClosed);
        assert_eq!(chat.name(), None);
    }

    fn device(name: &str) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(format!("id:{name}")),
            name: name.into(),
            is_default: false,
        }
    }

    fn with_second(settings: Settings) -> Settings {
        Settings {
            soundboard_device: Some(DeviceId("cable".into())),
            ..settings
        }
    }

    #[test]
    fn virtual_cables_are_recognised_by_name() {
        for name in [
            "CABLE Input (VB-Audio Virtual Cable)",
            "VoiceMeeter Input (VB-Audio VoiceMeeter VAIO)",
            "Voicemeeter AUX Input",
            "Line 1 (Virtual Audio Cable)",
            "cable input",
        ] {
            assert!(is_virtual_cable(name), "{name}");
        }
        for name in [
            "Speakers (Realtek(R) Audio)",
            "Headphones",
            "Наушники (Realtek(R) Audio)",
            "HDMI cable TV",
        ] {
            assert!(!is_virtual_cable(name), "{name}");
        }
    }

    #[test]
    fn the_first_cable_is_found() {
        let devices = [
            device("Speakers (Realtek(R) Audio)"),
            device("CABLE Input (VB-Audio Virtual Cable)"),
            device("VoiceMeeter Input (VB-Audio VoiceMeeter VAIO)"),
        ];
        assert_eq!(find_virtual_cable(&devices), Some(&devices[1]));
        assert_eq!(find_virtual_cable(&devices[..1]), None);
        assert_eq!(find_virtual_cable(&[]), None);
    }

    #[test]
    fn the_recording_side_hint_follows_the_cable() {
        assert_eq!(
            cable_side("CABLE Input (VB-Audio Virtual Cable)"),
            CableSide::VbCable
        );
        assert_eq!(cable_side("Line 1 (Virtual Audio Cable)"), CableSide::Other);
        assert_eq!(cable_side("Voicemeeter Input"), CableSide::Other);
    }

    #[test]
    fn the_microphone_stays_closed_without_a_second_device() {
        let mut chat = Chat::default();
        let settings = Settings {
            mic_enabled: true,
            share_music: true,
            ..Settings::default()
        };

        let (commands, _) = chat.on_settings(&settings);

        assert!(commands.contains(&EngineCommand::SetMicrophone {
            on: false,
            device: None,
            gain: 1.0,
        }));
        assert!(commands.contains(&EngineCommand::SetShareMusic {
            on: false,
            gain: 0.5,
        }));
    }

    #[test]
    fn share_music_is_sent_at_startup_and_on_change() {
        let mut chat = Chat::default();
        let settings = with_second(Settings {
            share_music: true,
            ..Settings::default()
        });

        let (commands, _) = chat.on_settings(&settings);
        assert!(commands.contains(&EngineCommand::SetShareMusic {
            on: true,
            gain: 0.5,
        }));
        // Nothing changed, nothing is sent.
        assert!(chat.on_settings(&settings).0.is_empty());

        let quieter = Settings {
            share_music_volume: 0.2,
            ..settings.clone()
        };
        assert_eq!(
            chat.on_settings(&quieter).0,
            [EngineCommand::SetShareMusic {
                on: true,
                gain: 0.2,
            }]
        );
        let no_second = Settings {
            soundboard_device: None,
            ..quieter
        };
        let commands = chat.on_settings(&no_second).0;
        assert!(commands.contains(&EngineCommand::SetShareMusic {
            on: false,
            gain: 0.2,
        }));
    }

    #[test]
    fn the_microphone_state_follows_the_engine() {
        let mut chat = Chat::default();
        let on = with_second(Settings {
            mic_enabled: true,
            mic_device: Some(DeviceId("mic".into())),
            mic_volume: 1.5,
            ..Settings::default()
        });
        let (commands, _) = chat.on_settings(&on);
        assert!(commands.contains(&EngineCommand::SetMicrophone {
            on: true,
            device: Some(DeviceId("mic".into())),
            gain: 1.5,
        }));
        assert_eq!(chat.state(), MicState::Off);

        assert_eq!(chat.on_engine(opened()), (true, None));
        assert_eq!(chat.state(), MicState::On);
        assert_eq!(
            chat.on_engine(EngineEvent::MicrophoneFailed { denied: false }),
            (true, Some(MICROPHONE_FAILED))
        );
        assert_eq!(chat.state(), MicState::Failed);
        assert_eq!(chat.on_engine(opened()), (true, None));
        assert_eq!(chat.on_engine(EngineEvent::MicrophoneClosed), (true, None));
        assert_eq!(chat.state(), MicState::Off);
        assert_eq!(chat.on_engine(EngineEvent::Stopped), (false, None));
    }

    #[test]
    fn a_denied_microphone_gives_the_denied_notice() {
        let mut chat = Chat::default();
        chat.on_settings(&with_second(Settings {
            mic_enabled: true,
            ..Settings::default()
        }));

        assert_eq!(
            chat.on_engine(EngineEvent::MicrophoneFailed { denied: true }),
            (true, Some(MICROPHONE_DENIED))
        );
    }

    #[test]
    fn a_cable_as_the_microphone_gives_its_notice() {
        let mut chat = Chat::default();
        chat.on_settings(&with_second(Settings {
            mic_enabled: true,
            ..Settings::default()
        }));

        assert_eq!(
            chat.on_engine(EngineEvent::MicrophoneIsCable),
            (true, Some(MICROPHONE_IS_CABLE))
        );
        assert_eq!(chat.state(), MicState::Failed);
    }

    #[test]
    fn cable_recording_sides_are_recognised_too() {
        for name in [
            "CABLE Output (VB-Audio Virtual Cable)",
            "VoiceMeeter Output (VB-Audio VoiceMeeter VAIO)",
            "Line 1 (Virtual Audio Cable)",
        ] {
            assert!(is_virtual_cable(name), "{name}");
        }
        assert!(!is_virtual_cable("Microphone (Realtek(R) Audio)"));
    }

    #[test]
    fn turning_it_off_clears_a_failure_at_once() {
        let mut chat = Chat::default();
        let on = with_second(Settings {
            mic_enabled: true,
            ..Settings::default()
        });
        chat.on_settings(&on);
        chat.on_engine(EngineEvent::MicrophoneFailed { denied: false });

        let off = Settings {
            mic_enabled: false,
            ..on
        };
        let (_, changed) = chat.on_settings(&off);
        assert!(changed);
        assert_eq!(chat.state(), MicState::Off);
        // The close the engine sends after that changes nothing, and says nothing.
        assert_eq!(chat.on_engine(EngineEvent::MicrophoneClosed), (false, None));
        assert_eq!(
            chat.on_engine(EngineEvent::MicrophoneFailed { denied: false }),
            (false, None)
        );
    }
}
