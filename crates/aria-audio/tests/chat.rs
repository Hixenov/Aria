//! The second output as a chat's microphone: the music copied into it, and the microphone.
//!
//! Driven through fake outputs and a fake capture, so no device and no real microphone is used.
//! Nothing plays until a test pulls frames.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use aria_audio::fake::{FakeCapture, FakeCaptureControl, FakeControl, FakeFactory};
use aria_audio::testing::write_tone_wav;
use aria_audio::{
    CaptureFormat, ClockReading, DeviceId, DeviceInfo, Engine, EngineCommand, EngineEvent,
    EngineHandle, FADE_MS, IDLE_GRACE, OutputFormat, OutputPath, OutputState, SoundRoute,
    TrackDecoder, TrackRequest,
};
use tempfile::TempDir;

use EngineCommand::{
    Load, Pause, Play, PlaySound, Seek, SetDevice, SetExclusive, SetMicrophone, SetShareMusic,
    SetSoundDevice, SetVolume, StopSounds,
};
use EngineEvent::{
    MicrophoneClosed, MicrophoneFailed, MicrophoneOpened, Output, SoundEnded, SoundFailed,
    SoundStarted,
};

const STEREO_44K: OutputFormat = OutputFormat {
    sample_rate: 44_100,
    channels: 2,
};
const STEREO_48K: OutputFormat = OutputFormat {
    sample_rate: 48_000,
    channels: 2,
};
/// Only a failing test waits this long.
const DEADLINE: Duration = Duration::from_secs(20);
const SECOND: usize = 44_100;
/// Samples at the start of what is heard in which a fade can still be heard.
const FADE: usize = 2 * SECOND * FADE_MS as usize / 1_000;
const SHORT_GRACE: Duration = Duration::from_millis(300);

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(1));
    }
}

fn second_device() -> DeviceId {
    DeviceId("cable".into())
}

/// A 16-bit stereo 440 Hz tone of `frames` frames at 44.1 kHz, in a folder that goes with the
/// guard.
fn tone(frames: u64) -> io::Result<(TempDir, PathBuf)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tone.wav");
    write_tone_wav(&path, 44_100, 2, frames, 440.0)?;
    Ok((dir, path))
}

fn decode_all(path: &Path) -> Vec<f32> {
    let mut decoder = match TrackDecoder::open(path) {
        Ok(decoder) => decoder,
        Err(error) => panic!("{} did not open: {error}", path.display()),
    };
    let mut samples = Vec::new();
    loop {
        match decoder.next_block() {
            Ok(Some(block)) => samples.extend_from_slice(block),
            Ok(None) => return samples,
            Err(error) => panic!("{} did not decode: {error}", path.display()),
        }
    }
}

/// Frames in which something can be heard.
fn audible_frames(samples: &[f32], channels: usize) -> usize {
    samples
        .chunks_exact(channels)
        .filter(|frame| frame.iter().any(|sample| *sample != 0.0))
        .count()
}

fn scaled(samples: &[f32], gain: f32) -> Vec<f32> {
    samples.iter().map(|sample| sample * gain).collect()
}

fn share(gain: f32) -> EngineCommand {
    SetShareMusic { on: true, gain }
}

/// The fake microphone, unless a test asks for another: the outputs' own format, so that nothing
/// is resampled.
const MIC_44K: CaptureFormat = CaptureFormat {
    sample_rate: 44_100,
    channels: 2,
};

struct Rig {
    factory: FakeFactory,
    engine: EngineHandle,
    events: Receiver<EngineEvent>,
    mic: FakeCaptureControl,
}

impl Rig {
    fn new(format: OutputFormat) -> Self {
        Self::with_idle_grace(format, IDLE_GRACE)
    }

    fn with_idle_grace(format: OutputFormat, grace: Duration) -> Self {
        Self::with_microphone(format, grace, MIC_44K)
    }

    fn with_microphone(format: OutputFormat, grace: Duration, mic: CaptureFormat) -> Self {
        let factory = FakeFactory::new(format);
        let (capture, mic) = FakeCapture::new(mic);
        let (sender, events) = mpsc::channel();
        let on_event = move |event| {
            let _ = sender.send(event);
        };
        let engine = match Engine::start_with_idle_grace(
            Box::new(factory.clone()),
            Box::new(capture),
            Box::new(on_event),
            grace,
        ) {
            Ok(engine) => engine,
            Err(error) => panic!("the engine did not start: {error}"),
        };
        Self {
            factory,
            engine,
            events,
            mic,
        }
    }

    /// Wait for `wanted`, passing over other events.
    fn expect(&self, wanted: EngineEvent) {
        loop {
            if self.next_event() == wanted {
                return;
            }
        }
    }

    fn send(&self, command: EngineCommand) {
        self.engine.send(command);
    }

    fn read(&self) -> ClockReading {
        self.engine.clock().read()
    }

    /// Pause, and return once the engine has filled the rings as far as it can.
    fn hold(&self, token: u64) {
        self.send(Pause);
        wait_until("the pause", || {
            let reading = self.read();
            reading.token == token && !reading.playing
        });
    }

    fn resume(&self) {
        self.send(Play);
        wait_until("playback", || self.read().playing);
    }

    /// Start `path` with the rings already full.
    fn start(&self, token: u64, path: &Path) {
        self.send(Load {
            track: TrackRequest {
                token,
                path: path.to_path_buf(),
            },
            start_ms: 0,
            play: true,
        });
        self.hold(token);
        self.resume();
    }

    /// The latest output on `device`, once one has opened. `None` is the system default.
    fn output_on(&self, device: Option<&DeviceId>) -> FakeControl {
        wait_until("an output on the device", || {
            !self.factory.outputs_on(device).is_empty()
        });
        match self.factory.outputs_on(device).pop() {
            Some(output) => output,
            None => panic!("the output went away"),
        }
    }

    /// The latest output on the second device, once the engine has put something on it (the
    /// copy, the microphone or a sound). It is opened on a thread of its own, so the factory has
    /// it a moment before the engine does.
    fn second_in_use(&self) -> FakeControl {
        let second = self.output_on(Some(&second_device()));
        wait_until("the engine to use the second output", || {
            second.inputs_added() > 0
        });
        second
    }

    fn second_outputs(&self) -> usize {
        self.factory.outputs_on(Some(&second_device())).len()
    }

    fn next_event(&self) -> EngineEvent {
        match self.events.recv_timeout(DEADLINE) {
            Ok(event) => event,
            Err(error) => panic!("no event came: {error}"),
        }
    }

    /// Every event sent so far, without waiting.
    fn drain_events(&self) -> Vec<EngineEvent> {
        self.events.try_iter().collect()
    }

    /// Let the engine run a few passes.
    fn settle(&self) {
        thread::sleep(Duration::from_millis(50));
    }
}

/// Pulls `from` in small steps, pulling `beside` along with it when given (the copy is fed only
/// with what the music's output takes in), until `frames` stereo frames have come after the
/// leading silence. Returns those frames. On a slow machine the audio comes later, not less.
fn pull_sounding(from: &FakeControl, beside: Option<&FakeControl>, frames: usize) -> Vec<f32> {
    let deadline = Instant::now() + DEADLINE;
    let mut heard: Vec<f32> = Vec::new();
    loop {
        if let Some(beside) = beside {
            beside.pull(SECOND / 100);
        }
        heard.extend(from.pull(SECOND / 100));
        let start = heard
            .as_chunks::<2>()
            .0
            .iter()
            .position(|frame| frame.iter().any(|sample| *sample != 0.0));
        if let Some(start) = start
            && heard.len() / 2 - start >= frames
        {
            return heard[2 * start..2 * (start + frames)].to_vec();
        }
        assert!(Instant::now() < deadline, "only silence came");
        thread::sleep(Duration::from_millis(5));
    }
}

/// A rig sharing the music at `gain` to the second device, with `path` playing and its rings full.
fn sharing(gain: f32, path: &Path) -> (Rig, FakeControl, FakeControl) {
    let rig = Rig::new(STEREO_44K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(share(gain));
    rig.start(1, path);
    let mine = rig.output_on(None);
    let second = rig.second_in_use();
    (rig, mine, second)
}

#[test]
fn sharing_sends_the_music_to_the_second_output_at_its_gain() {
    let (_dir, path) = tone(SECOND as u64).unwrap();
    let music = decode_all(&path);
    let (_rig, mine, second) = sharing(0.5, &path);

    mine.pull(SECOND / 4);
    let copy = second.pull(SECOND / 4);

    assert!(
        copy[FADE..] == scaled(&music[FADE..copy.len()], 0.5)[..],
        "the copy is not the music at half level"
    );
}

#[test]
fn sharing_leaves_the_local_output_unchanged() {
    let (_dir, path) = tone(SECOND as u64).unwrap();
    let heard = |share_it: bool| {
        let rig = Rig::new(STEREO_44K);
        rig.send(SetSoundDevice(Some(second_device())));
        rig.send(SetShareMusic {
            on: share_it,
            gain: 0.5,
        });
        rig.start(1, &path);
        let mine = rig.output_on(None);
        let audio = mine.pull(SECOND / 2);
        assert_eq!(rig.second_outputs(), usize::from(share_it));
        audio
    };

    assert!(
        heard(true) == heard(false),
        "sharing changed the local output"
    );
}

#[test]
fn no_copy_while_sharing_is_off() {
    let (_dir, path) = tone(SECOND as u64).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(SetShareMusic {
        on: false,
        gain: 0.5,
    });
    rig.start(1, &path);
    rig.output_on(None).pull(SECOND / 4);
    rig.settle();

    assert_eq!(rig.second_outputs(), 0);
}

#[test]
fn turning_sharing_off_takes_the_copy_away() {
    let (_dir, path) = tone(2 * SECOND as u64).unwrap();
    let (rig, mine, second) = sharing(1.0, &path);
    mine.pull(SECOND / 4);
    second.pull(SECOND / 4);

    rig.send(SetShareMusic {
        on: false,
        gain: 1.0,
    });
    rig.settle();
    mine.pull(SECOND / 4);
    second.pull(SECOND / 10);

    assert_eq!(audible_frames(&second.pull(SECOND / 4), 2), 0);
}

#[test]
fn no_copy_without_a_second_device() {
    let (_dir, path) = tone(SECOND as u64).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.send(share(1.0));
    rig.start(1, &path);
    rig.output_on(None).pull(SECOND / 4);
    rig.settle();

    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn no_copy_when_the_second_device_is_the_local_device() {
    let (_dir, path) = tone(SECOND as u64).unwrap();
    let music = decode_all(&path);

    // The same device chosen for both.
    let rig = Rig::new(STEREO_44K);
    rig.send(SetDevice(Some(second_device())));
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(share(1.0));
    rig.start(1, &path);
    let output = rig.output_on(Some(&second_device()));
    let heard = output.pull(SECOND / 2);
    rig.settle();
    assert_eq!(rig.factory.outputs().len(), 1);
    // Once: a copy on the same output would double the level.
    assert!(
        heard[FADE..] == music[FADE..heard.len()],
        "the music changed"
    );

    // The music on the system default, which the second device is.
    let rig = Rig::new(STEREO_44K);
    rig.factory.set_devices(vec![DeviceInfo {
        id: second_device(),
        name: "Speakers".into(),
        is_default: true,
    }]);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(share(1.0));
    rig.start(1, &path);
    rig.output_on(None).pull(SECOND / 4);
    rig.settle();
    assert_eq!(rig.factory.outputs().len(), 1);
}

/// A rig whose chosen music device is missing, so the music falls back to the system default,
/// which is the second device.
fn fallen_back_onto_the_second_device() -> Rig {
    let rig = Rig::new(STEREO_44K);
    rig.factory.set_devices(vec![DeviceInfo {
        id: second_device(),
        name: "Speakers".into(),
        is_default: true,
    }]);
    let usb = DeviceId("usb".into());
    rig.factory.remove_device(usb.clone());
    rig.send(SetDevice(Some(usb)));
    rig.send(SetSoundDevice(Some(second_device())));
    rig
}

#[test]
fn no_copy_when_the_music_falls_back_onto_the_second_device() {
    let (_dir, path) = tone(SECOND as u64).unwrap();
    let rig = fallen_back_onto_the_second_device();
    rig.send(share(1.0));
    rig.start(1, &path);
    rig.output_on(None).pull(SECOND / 4);
    rig.settle();
    rig.output_on(None).pull(SECOND / 4);
    rig.settle();
    let open: Vec<_> = rig
        .factory
        .outputs()
        .into_iter()
        .filter(|output| !output.is_closed())
        .collect();
    assert_eq!(
        open.len(),
        1,
        "a copy went to the device the music falls back to"
    );
}

#[test]
fn the_microphone_closes_when_the_music_falls_back_onto_the_second_device() {
    let (_dir, path) = tone(SECOND as u64).unwrap();
    let rig = fallen_back_onto_the_second_device();
    rig.send(microphone(true, 1.0));
    rig.start(1, &path);
    rig.output_on(None).pull(SECOND / 4);
    rig.settle();
    wait_until("the microphone to close", || !rig.mic.open_now());
    assert!(
        rig.factory
            .outputs_on(Some(&second_device()))
            .iter()
            .all(FakeControl::is_closed),
        "the second output stayed open on the music's device"
    );
}

#[test]
fn local_mute_and_volume_do_not_touch_the_copy() {
    let (_dir, path) = tone(SECOND as u64).unwrap();
    let music = decode_all(&path);
    let rig = Rig::new(STEREO_44K);
    rig.send(SetVolume(0.0));
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(share(0.5));
    rig.start(1, &path);
    let mine = rig.output_on(None);
    let second = rig.output_on(Some(&second_device()));

    let local = mine.pull(SECOND / 4);
    let copy = second.pull(SECOND / 4);

    assert_eq!(audible_frames(&local, 2), 0, "the muted output sounds");
    assert!(
        copy[FADE..] == scaled(&music[FADE..copy.len()], 0.5)[..],
        "the copy followed the local volume"
    );
}

#[test]
fn pause_and_seek_act_on_the_copy() {
    let (_dir, path) = tone(2 * SECOND as u64).unwrap();
    let music = decode_all(&path);
    let (rig, mine, second) = sharing(1.0, &path);
    mine.pull(SECOND / 10);
    second.pull(SECOND / 10);

    rig.send(Pause);
    wait_until("the pause", || !rig.read().playing);
    mine.pull(SECOND / 10);
    second.pull(SECOND / 100);
    assert_eq!(
        audible_frames(&second.pull(SECOND / 10), 2),
        0,
        "the copy plays on while paused"
    );

    // 1.01 s in, where the tone is not at a zero, so the first frame of the copy sounds.
    rig.send(Seek(1_010));
    rig.send(Play);
    wait_until("playback", || rig.read().playing);
    // Each output acts on the seek when it is next pulled, and then gets the audio from there.
    // A quarter of a second of samples, in stereo frames.
    let sounding = pull_sounding(&second, Some(&mine), SECOND / 8 + 1);
    let from = 2 * 44_541;
    let check = &sounding[FADE..SECOND / 4];
    let expected = &music[from + FADE..from + SECOND / 4];
    assert!(check == expected, "the copy did not seek with the music");
}

#[test]
fn the_copy_is_resampled_to_the_second_output() {
    let (_dir, path) = tone(3 * SECOND as u64).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.factory.set_shared_format(second_device(), STEREO_48K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(share(1.0));
    rig.start(1, &path);
    let mine = rig.output_on(None);
    let second = rig.output_on(Some(&second_device()));
    assert_eq!(second.opened_as().1.sample_rate, 48_000);

    // Both outputs run side by side until half a second of the copy has been heard. How the
    // audio arrives depends on how busy the machine is; its pitch does not.
    let deadline = Instant::now() + DEADLINE;
    let mut copy = Vec::new();
    while audible_frames(&copy, 2) < 24_000 {
        assert!(Instant::now() < deadline, "the copy did not come");
        mine.pull(4_410);
        copy.extend(second.pull(4_800));
        thread::sleep(Duration::from_millis(5));
    }

    // The 440 Hz tone crosses zero 880 times a second at the second output's 48 kHz. Played at
    // 44.1 kHz unconverted, it would cross about 958 times in 48,000 frames.
    let left: Vec<f32> = copy
        .iter()
        .step_by(2)
        .copied()
        .filter(|sample| *sample != 0.0)
        .collect();
    let second_of_it = &left[2_000..2_000 + 20_000];
    let crossings = second_of_it
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count();
    // 20,000 frames at 48 kHz hold 366.7 crossings of a 440 Hz tone.
    assert!((360..=373).contains(&crossings), "{crossings} crossings");
}

#[test]
fn a_full_copy_ring_never_stalls_the_music() {
    let (_dir, path) = tone(5 * SECOND as u64).unwrap();
    let (_rig, mine, _second) = sharing(1.0, &path);

    // The second output is never pulled, so the copy's ring fills and stays full, and the music
    // must still come: four seconds of it, at whatever pace a busy machine refills the ring.
    let mut sounding = 0;
    let deadline = Instant::now() + DEADLINE;
    while sounding < 4 * SECOND {
        sounding += audible_frames(&mine.pull(SECOND / 20), 2);
        thread::sleep(Duration::from_millis(10));
        assert!(
            Instant::now() < deadline,
            "the music stalled at {sounding} frames"
        );
    }
}

#[test]
fn a_reopen_of_the_local_output_pauses_the_copy_and_it_carries_on() {
    let (_dir, path) = tone(4 * SECOND as u64).unwrap();
    let (rig, mine, second) = sharing(1.0, &path);
    mine.pull(SECOND / 4);
    second.pull(SECOND / 4);

    mine.fail();
    wait_until("a new local output", || {
        rig.factory.outputs_on(None).len() == 2
    });
    let reopened = rig.output_on(None);
    reopened.pull(SECOND / 4);
    rig.settle();
    let copy = second.pull(SECOND / 2);

    assert!(!second.is_closed(), "the second output was closed");
    assert_eq!(rig.second_outputs(), 1);
    // The copy goes on from where the ear was, so less than the queued second is repeated.
    let sounding = audible_frames(&copy, 2);
    assert!(
        sounding > SECOND / 8,
        "the copy did not carry on: {sounding}"
    );
}

#[test]
fn exclusive_stays_bit_perfect_while_sharing() {
    let (_dir, path) = tone(SECOND as u64).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.send(SetExclusive(true));
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(share(0.5));
    rig.start(1, &path);
    let mine = rig.output_on(None);
    let second = rig.output_on(Some(&second_device()));

    let state = loop {
        if let Output(state) = rig.next_event() {
            break state;
        }
    };
    assert_eq!(
        state,
        OutputState {
            path: OutputPath::BitPerfect,
            device_rate: 44_100,
            device_bits: 16,
        }
    );
    assert!(mine.opened_as().0, "the local output is not exclusive");
    assert!(!second.opened_as().0, "the second output is exclusive");
    mine.pull(SECOND / 4);
    assert!(audible_frames(&second.pull(SECOND / 4), 2) > 0);
    rig.settle();
    assert!(
        !rig.drain_events().iter().any(|e| matches!(e, Output(_))),
        "the state changed"
    );
}

#[test]
fn sounds_and_the_copy_share_the_second_output() {
    let (_dir, path) = tone(2 * SECOND as u64).unwrap();
    let sound_dir = tempfile::tempdir().unwrap();
    let sound = sound_dir.path().join("sound.wav");
    write_tone_wav(&sound, 44_100, 2, (SECOND / 4) as u64, 660.0).unwrap();
    let music = decode_all(&path);
    let sound_samples = decode_all(&sound);
    let (rig, mine, second) = sharing(0.5, &path);
    mine.pull(SECOND / 10);
    let lead = second.pull(SECOND / 10);

    rig.send(PlaySound {
        token: 3,
        path: sound,
        gain: 1.0,
        route: SoundRoute::Second,
    });
    loop {
        if rig.next_event() == (SoundStarted { token: 3 }) {
            break;
        }
    }
    let mixed = second.pull(SECOND / 8);

    let at = lead.len();
    let expected: Vec<f32> = music[at..at + mixed.len()]
        .iter()
        .zip(&sound_samples)
        .map(|(music, sound)| music * 0.5 + sound)
        .collect();
    assert!(
        mixed == expected,
        "the second output is not the copy plus the sound"
    );
    assert_eq!(rig.second_outputs(), 1);
    second.pull(SECOND / 4);
    loop {
        if rig.next_event() == (SoundEnded { token: 3 }) {
            break;
        }
    }
}

#[test]
fn the_second_output_stays_open_while_music_is_shared_and_playing() {
    let (_dir, path) = tone(10 * SECOND as u64).unwrap();
    let rig = Rig::with_idle_grace(STEREO_44K, SHORT_GRACE);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(share(1.0));
    rig.start(1, &path);
    let mine = rig.output_on(None);
    let second = rig.output_on(Some(&second_device()));

    // Three grace periods of playing.
    let until = Instant::now() + 3 * SHORT_GRACE;
    while Instant::now() < until {
        mine.pull(SECOND / 100);
        second.pull(SECOND / 100);
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !second.is_closed(),
        "the second output closed while sharing"
    );

    // Paused, the copy needs it no more.
    rig.send(Pause);
    wait_until("the second output to close", || second.is_closed());
}

// The microphone.

fn microphone(on: bool, gain: f32) -> EngineCommand {
    SetMicrophone {
        on,
        device: None,
        gain,
    }
}

/// A rig with the microphone on at `gain` into the second device, open.
fn talking(gain: f32) -> (Rig, FakeControl) {
    let rig = Rig::new(STEREO_44K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, gain));
    rig.expect(MicrophoneOpened { name: None });
    let second = rig.output_on(Some(&second_device()));
    (rig, second)
}

#[test]
fn the_opened_microphone_is_named() {
    let rig = Rig::new(STEREO_44K);
    rig.mic.set_devices(vec![DeviceInfo {
        id: DeviceId("usb".into()),
        name: "USB Microphone".into(),
        is_default: true,
    }]);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, 1.0));
    rig.expect(MicrophoneOpened {
        name: Some("USB Microphone".into()),
    });
}

#[test]
fn a_slow_microphone_does_not_hold_up_the_music() {
    let (_dir, path) = tone(5 * SECOND as u64).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.start(1, &path);
    let mine = rig.output_on(None);
    mine.pull(SECOND / 10);
    rig.settle();

    // A microphone that takes two seconds to open, longer than the music's one-second ring.
    rig.mic.delay_open(Duration::from_secs(2));
    rig.send(microphone(true, 1.0));
    // Pulled in real time for a second and a half, the music never runs dry.
    let chunk = SECOND / 10;
    for step in 0..15 {
        thread::sleep(Duration::from_millis(100));
        let heard = mine.pull(chunk);
        assert!(
            audible_frames(&heard, 2) * 10 >= chunk * 9,
            "the music ran dry {} ms after the microphone was asked for",
            (step + 1) * 100
        );
    }
    rig.expect(MicrophoneOpened { name: None });
}

#[test]
fn a_microphone_turned_off_while_it_opens_stays_closed() {
    let rig = Rig::new(STEREO_44K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.mic.delay_open(Duration::from_millis(400));
    rig.send(microphone(true, 1.0));
    wait_until("the open to begin", || rig.mic.asked() == 1);
    rig.send(microphone(false, 1.0));
    // Once the open has finished, it is closed again and nothing was said of it.
    wait_until("the late open to finish", || rig.mic.opened() == 1);
    wait_until("the late microphone to close", || !rig.mic.open_now());
    rig.settle();
    let events = rig.drain_events();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, MicrophoneOpened { .. })),
        "{events:?}"
    );
}

/// `frames` frames of a steady `level` in `channels` channels.
fn steady(level: f32, frames: usize, channels: usize) -> Vec<f32> {
    vec![level; frames * channels]
}

/// Push `samples` into the microphone and wait until the engine has taken them, then give it a
/// moment to move them on.
fn speak(rig: &Rig, samples: &[f32]) {
    assert_eq!(
        rig.mic.push(samples),
        samples.len(),
        "the capture ring is full"
    );
    wait_until("the engine to take the microphone's samples", || {
        rig.mic.waiting() == 0
    });
    rig.settle();
}

fn mic_events(events: &[EngineEvent]) -> Vec<EngineEvent> {
    events
        .iter()
        .filter(|event| {
            matches!(
                event,
                MicrophoneOpened { .. } | MicrophoneClosed | MicrophoneFailed { .. }
            )
        })
        .cloned()
        .collect()
}

#[test]
fn the_microphone_goes_to_the_second_output_at_its_gain() {
    let (rig, second) = talking(0.5);

    speak(&rig, &steady(0.25, 441, 2));
    let heard = second.pull(882);

    assert!(
        heard[..882] == steady(0.125, 441, 2)[..],
        "{:?}",
        &heard[..4]
    );
    assert_eq!(audible_frames(&heard[882..], 2), 0);
}

#[test]
fn the_microphone_never_reaches_the_local_output() {
    let (_dir, path) = tone(SECOND as u64).unwrap();
    let music = decode_all(&path);
    let (rig, second) = talking(1.0);
    rig.start(1, &path);
    let mine = rig.output_on(None);

    speak(&rig, &steady(0.25, 4_410, 2));
    let local = mine.pull(SECOND / 4);
    second.pull(4_410);

    assert!(
        local[FADE..] == music[FADE..local.len()],
        "the microphone is in the local output"
    );
}

#[test]
fn a_mono_microphone_reaches_both_channels() {
    let mono = CaptureFormat {
        sample_rate: 44_100,
        channels: 1,
    };
    let rig = Rig::with_microphone(STEREO_44K, IDLE_GRACE, mono);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, 1.0));
    rig.expect(MicrophoneOpened { name: None });
    let second = rig.output_on(Some(&second_device()));

    speak(&rig, &steady(0.25, 441, 1));
    let heard = second.pull(441);

    assert!(heard == steady(0.25, 441, 2), "{:?}", &heard[..4]);
}

#[test]
fn gain_above_one_amplifies_and_clips() {
    let (rig, second) = talking(2.0);

    speak(&rig, &steady(0.25, 441, 2));
    assert!(second.pull(441) == steady(0.5, 441, 2));

    speak(&rig, &steady(0.75, 441, 2));
    assert!(second.pull(441) == steady(1.0, 441, 2), "not clipped");
}

#[test]
fn turning_the_microphone_off_closes_the_capture() {
    let (rig, _second) = talking(1.0);
    assert!(rig.mic.open_now());

    rig.send(microphone(false, 1.0));
    rig.expect(MicrophoneClosed);

    assert!(!rig.mic.open_now());
    rig.settle();
    assert_eq!(rig.mic.opened(), 1);
}

#[test]
fn no_second_device_opens_no_microphone() {
    let rig = Rig::new(STEREO_44K);
    rig.send(microphone(true, 1.0));
    rig.settle();

    assert_eq!(rig.mic.opened(), 0);
    assert!(rig.factory.outputs().is_empty());
    assert!(mic_events(&rig.drain_events()).is_empty());
}

#[test]
fn no_microphone_when_the_second_device_is_the_local_device() {
    // The same device chosen for both.
    let rig = Rig::new(STEREO_44K);
    rig.send(SetDevice(Some(second_device())));
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, 1.0));
    rig.settle();
    assert_eq!(rig.mic.opened(), 0);

    // The music on the system default, which the second device is.
    let rig = Rig::new(STEREO_44K);
    rig.factory.set_devices(vec![DeviceInfo {
        id: second_device(),
        name: "Speakers".into(),
        is_default: true,
    }]);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, 1.0));
    rig.settle();
    assert_eq!(rig.mic.opened(), 0);

    // Moving the music onto the second device closes the microphone.
    let (rig, _second) = talking(1.0);
    rig.send(SetDevice(Some(second_device())));
    rig.expect(MicrophoneClosed);
    assert!(!rig.mic.open_now());
}

#[test]
fn the_microphone_never_queues_more_than_40_ms() {
    let (rig, second) = talking(1.0);

    // A whole second at once, as after a stall: only the newest 40 ms is kept.
    let second_of_speech: Vec<f32> = (0..SECOND)
        .flat_map(|frame| {
            let level = if frame < SECOND - 1_764 { 0.25 } else { 0.5 };
            [level, level]
        })
        .collect();
    speak(&rig, &second_of_speech);
    let heard = second.pull(SECOND);

    let sounding = audible_frames(&heard, 2);
    assert!(sounding <= 1_764, "{sounding} frames queued, over 40 ms");
    assert!(sounding > 1_000, "only {sounding} frames heard");
    // What is kept is the newest audio.
    assert!(heard[..2 * sounding].iter().all(|sample| *sample == 0.5));
}

#[test]
fn steady_speech_is_not_dropped() {
    let (rig, second) = talking(1.0);

    let mut heard = Vec::new();
    for _ in 0..30 {
        speak(&rig, &steady(0.25, 441, 2));
        heard.extend(second.pull(441));
    }
    heard.extend(second.pull(2_000));

    assert_eq!(audible_frames(&heard, 2), 30 * 441);
}

#[test]
fn a_failed_capture_reports_and_music_and_sounds_go_on() {
    let (_dir, path) = tone(SECOND as u64).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.mic.fail_open(true);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, 1.0));
    rig.expect(MicrophoneFailed { denied: false });

    rig.start(1, &path);
    let mine = rig.output_on(None);
    assert!(audible_frames(&mine.pull(SECOND / 4), 2) > SECOND / 8);
    assert!(!rig.mic.open_now());
}

#[test]
fn a_denied_capture_reports_denied() {
    let rig = Rig::new(STEREO_44K);
    rig.mic.deny(true);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, 1.0));

    rig.expect(MicrophoneFailed { denied: true });
}

#[test]
fn a_lost_capture_reports_failed_and_retries_on_the_next_change() {
    let (rig, _second) = talking(1.0);

    rig.mic.fail();
    rig.expect(MicrophoneFailed { denied: false });
    assert!(!rig.mic.open_now());
    rig.settle();
    assert_eq!(rig.mic.opened(), 1, "retried without a change");

    rig.send(microphone(true, 1.0));
    rig.expect(MicrophoneOpened { name: None });
    assert_eq!(rig.mic.opened(), 2);
}

#[test]
fn the_second_output_stays_open_while_the_microphone_is_on() {
    let rig = Rig::with_idle_grace(STEREO_44K, SHORT_GRACE);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, 1.0));
    rig.expect(MicrophoneOpened { name: None });
    let second = rig.output_on(Some(&second_device()));

    thread::sleep(3 * SHORT_GRACE);
    assert!(
        !second.is_closed(),
        "the second output closed while talking"
    );

    rig.send(microphone(false, 1.0));
    wait_until("the second output to close", || second.is_closed());
}

#[test]
fn a_microphone_slower_than_the_idle_grace_still_opens() {
    let rig = Rig::with_idle_grace(STEREO_44K, SHORT_GRACE);
    rig.mic.delay_open(3 * SHORT_GRACE);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, 1.0));
    rig.expect(MicrophoneOpened { name: None });
    let second = rig.output_on(Some(&second_device()));
    assert!(
        !second.is_closed(),
        "the second output closed while the microphone opened"
    );
    assert_eq!(
        rig.second_outputs(),
        1,
        "the second output was closed and opened again"
    );
}

#[test]
fn the_microphone_is_resampled_to_the_second_output() {
    let mono_48k = CaptureFormat {
        sample_rate: 48_000,
        channels: 1,
    };
    let rig = Rig::with_microphone(STEREO_44K, IDLE_GRACE, mono_48k);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, 1.0));
    rig.expect(MicrophoneOpened { name: None });
    let second = rig.output_on(Some(&second_device()));

    // Half a second, 20 ms at a time.
    let mut heard = Vec::new();
    for _ in 0..25 {
        speak(&rig, &steady(0.25, 960, 1));
        heard.extend(second.pull(882));
    }
    heard.extend(second.pull(4_410));

    let sounding = audible_frames(&heard, 2);
    assert!(sounding > SECOND / 2 * 8 / 10, "only {sounding} frames");
    assert!(sounding < SECOND / 2 * 11 / 10, "{sounding} frames");
    // Where it sounds, away from the resampler's ramp at either end, it is the steady level. A
    // slow machine can leave silent gaps between pieces; the level around them stays.
    let sounding: Vec<f32> = heard
        .iter()
        .copied()
        .filter(|sample| *sample != 0.0)
        .collect();
    let middle = &sounding[2_000..sounding.len() - 2_000];
    assert!(
        middle.iter().all(|sample| (sample - 0.25).abs() < 0.01),
        "the level changed"
    );
}

#[test]
fn a_second_device_change_moves_the_microphone() {
    let (rig, _second) = talking(1.0);
    let other = DeviceId("other cable".into());

    rig.send(SetSoundDevice(Some(other.clone())));
    rig.expect(MicrophoneClosed);
    rig.expect(MicrophoneOpened { name: None });
    let moved = rig.output_on(Some(&other));

    speak(&rig, &steady(0.25, 441, 2));
    assert!(pull_sounding(&moved, None, 441) == steady(0.25, 441, 2));
}

#[test]
fn the_input_limit_holds_copy_microphone_and_six_sounds() {
    let (_dir, path) = tone(4 * SECOND as u64).unwrap();
    let sound_dir = tempfile::tempdir().unwrap();
    let sound = sound_dir.path().join("sound.wav");
    write_tone_wav(&sound, 44_100, 2, 2 * SECOND as u64, 660.0).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(share(0.5));
    rig.send(microphone(true, 1.0));
    rig.expect(MicrophoneOpened { name: None });
    rig.start(1, &path);
    let second = rig.output_on(Some(&second_device()));

    for token in 1..=7 {
        rig.send(PlaySound {
            token,
            path: sound.clone(),
            gain: 0.1,
            route: SoundRoute::Second,
        });
    }
    // Every sound is set up before the next command; wait for all seven to say so.
    let mut events = Vec::new();
    let deadline = Instant::now() + DEADLINE;
    while events
        .iter()
        .filter(|event| matches!(event, SoundStarted { .. }))
        .count()
        < 7
    {
        assert!(Instant::now() < deadline, "{events:?}");
        second.pull(441);
        events.extend(rig.drain_events());
        thread::sleep(Duration::from_millis(5));
    }

    let started = events
        .iter()
        .filter(|event| matches!(event, SoundStarted { .. }))
        .count();
    assert_eq!(started, 7);
    assert!(events.contains(&SoundEnded { token: 1 }), "{events:?}");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SoundFailed { .. })),
        "{events:?}"
    );
    speak(&rig, &steady(0.25, 441, 2));
    assert!(audible_frames(&second.pull(441), 2) > 0);
    assert_eq!(rig.second_outputs(), 1);
}

// Found in the final review.

#[test]
fn a_track_loaded_onto_an_open_output_is_shared() {
    let (_dir, path) = tone(2 * SECOND as u64).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.start(1, &path);
    let mine = rig.output_on(None);
    mine.pull(SECOND / 10);
    // Stopped, with the output still open: sharing is turned on while nothing is loaded.
    rig.send(EngineCommand::Stop);
    rig.send(share(1.0));
    rig.settle();
    assert_eq!(rig.second_outputs(), 0);

    // Only a Load, as the core sends for a double-click: no Play follows it.
    rig.send(Load {
        track: TrackRequest {
            token: 2,
            path: path.clone(),
        },
        start_ms: 0,
        play: true,
    });
    wait_until("playback", || rig.read().token == 2 && rig.read().playing);
    rig.output_on(None).pull(SECOND / 10);
    rig.settle();
    let second = rig.output_on(Some(&second_device()));
    rig.output_on(None).pull(SECOND / 4);
    rig.settle();

    assert!(audible_frames(&second.pull(SECOND / 4), 2) > SECOND / 8);
}

#[test]
fn the_end_of_the_queue_leaves_the_chat_silent_and_in_step() {
    let frames = SECOND / 2;
    let (_dir, path) = tone(frames as u64).unwrap();
    let music = decode_all(&path);
    let (rig, mine, second) = sharing(1.0, &path);
    // Played to the end, as both outputs run.
    let deadline = Instant::now() + DEADLINE;
    loop {
        mine.pull(SECOND / 10);
        second.pull(SECOND / 10);
        if rig.drain_events().contains(&EngineEvent::Stopped) {
            break;
        }
        assert!(Instant::now() < deadline, "no end");
        thread::sleep(Duration::from_millis(2));
    }
    second.pull(SECOND);

    // The core gets the last track ready again, without playing it.
    rig.send(Load {
        track: TrackRequest {
            token: 2,
            path: path.clone(),
        },
        start_ms: 0,
        play: false,
    });
    rig.settle();
    mine.pull(SECOND / 10);
    assert_eq!(
        audible_frames(&second.pull(SECOND / 4), 2),
        0,
        "the chat heard the track while stopped"
    );

    // Played again, the two start together.
    rig.send(Play);
    wait_until("playback", || rig.read().playing);
    let local = mine.pull(SECOND / 4);
    let copy = second.pull(SECOND / 4);
    assert!(
        local[FADE..] == music[FADE..local.len()],
        "the local output"
    );
    assert!(
        copy[FADE..] == music[FADE..copy.len()],
        "the copy is out of step"
    );
}

#[test]
fn sharing_turned_on_mid_track_leaves_the_local_output_unchanged() {
    let (_dir, path) = tone(3 * SECOND as u64).unwrap();
    let heard = |share_it: bool| {
        let rig = Rig::new(STEREO_44K);
        rig.send(SetSoundDevice(Some(second_device())));
        rig.start(1, &path);
        let mine = rig.output_on(None);
        let mut audio = mine.pull(SECOND / 4);
        if share_it {
            rig.send(share(1.0));
            rig.settle();
        }
        audio.extend(mine.pull(SECOND / 2));
        rig.settle();
        audio.extend(mine.pull(SECOND / 2));
        // Then both run on until a quarter of a second of the copy has been heard, however busy
        // the machine is.
        let copy = rig
            .factory
            .outputs_on(Some(&second_device()))
            .pop()
            .map(|second| {
                let (mut copy, deadline) = (Vec::new(), Instant::now() + DEADLINE);
                while audible_frames(&copy, 2) < SECOND / 4 && Instant::now() < deadline {
                    mine.pull(SECOND / 20);
                    copy.extend(second.pull(SECOND / 20));
                    thread::sleep(Duration::from_millis(5));
                }
                copy
            });
        (audio, copy)
    };

    let (plain, _) = heard(false);
    let (shared, copy) = heard(true);
    assert!(
        plain == shared,
        "turning sharing on changed the local output"
    );
    let copy = copy.unwrap_or_default();
    assert!(audible_frames(&copy, 2) >= SECOND / 4, "no copy");
}

#[test]
fn a_failed_second_output_fails_the_microphone_and_it_comes_back_with_it() {
    let (rig, second) = talking(1.0);

    second.fail();
    rig.expect(MicrophoneFailed { denied: false });
    assert!(!rig.mic.open_now());

    // A sound opens the second output again, and the microphone with it.
    let sound_dir = tempfile::tempdir().unwrap();
    let sound = sound_dir.path().join("sound.wav");
    write_tone_wav(&sound, 44_100, 2, (SECOND / 10) as u64, 660.0).unwrap();
    rig.send(PlaySound {
        token: 1,
        path: sound,
        gain: 1.0,
        route: SoundRoute::Second,
    });
    rig.expect(MicrophoneOpened { name: None });
    assert!(rig.mic.open_now());
    assert_eq!(rig.second_outputs(), 2);
}

#[test]
fn a_cable_as_the_microphone_is_refused() {
    let rig = Rig::new(STEREO_44K);
    rig.mic.set_devices(vec![DeviceInfo {
        id: DeviceId("cable out".into()),
        name: "CABLE Output (VB-Audio Virtual Cable)".into(),
        is_default: true,
    }]);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, 1.0));

    rig.expect(EngineEvent::MicrophoneIsCable);
    assert!(!rig.mic.open_now());
}

/// A 660 Hz sound of `frames` frames at 44.1 kHz, in a folder that goes with the guard.
fn sound_file(frames: usize) -> io::Result<(TempDir, PathBuf)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("sound.wav");
    write_tone_wav(&path, 44_100, 2, frames as u64, 660.0)?;
    Ok((dir, path))
}

/// Pulls `output` in real time for a second and a half, and fails if the music runs dry.
fn music_keeps_playing(output: &FakeControl, after: &str) {
    let chunk = SECOND / 10;
    for step in 0..15 {
        thread::sleep(Duration::from_millis(100));
        let heard = output.pull(chunk);
        assert!(
            audible_frames(&heard, 2) * 10 >= chunk * 9,
            "the music ran dry {} ms after {after}",
            (step + 1) * 100
        );
    }
}

#[test]
fn a_second_device_slow_to_open_does_not_hold_up_the_music() {
    let (_dir, path) = tone(5 * SECOND as u64).unwrap();
    let (_sound_dir, sound) = sound_file(SECOND / 4).unwrap();
    let rig = Rig::new(STEREO_44K);
    // Two seconds, longer than the music's one-second ring.
    rig.factory
        .slow_device(second_device(), Duration::from_secs(2), Duration::ZERO);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.start(1, &path);
    let mine = rig.output_on(None);
    mine.pull(SECOND / 10);
    rig.settle();

    rig.send(PlaySound {
        token: 3,
        path: sound,
        gain: 1.0,
        route: SoundRoute::Second,
    });
    music_keeps_playing(&mine, "the sound was fired");
    // The sound plays once the device is open.
    rig.expect(SoundStarted { token: 3 });
    let second = rig.output_on(Some(&second_device()));
    assert!(!pull_sounding(&second, None, SECOND / 20).is_empty());
}

#[test]
fn a_second_device_slow_to_close_does_not_hold_up_the_music() {
    let (_dir, path) = tone(5 * SECOND as u64).unwrap();
    let (_sound_dir, sound) = sound_file(SECOND / 4).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.factory
        .slow_device(second_device(), Duration::ZERO, Duration::from_secs(2));
    rig.send(SetSoundDevice(Some(second_device())));
    rig.start(1, &path);
    let mine = rig.output_on(None);
    rig.send(PlaySound {
        token: 3,
        path: sound,
        gain: 1.0,
        route: SoundRoute::Second,
    });
    rig.expect(SoundStarted { token: 3 });
    let second = rig.output_on(Some(&second_device()));
    mine.pull(SECOND / 10);
    rig.settle();

    // No second device: its output closes, which takes two seconds.
    rig.send(SetSoundDevice(None));
    music_keeps_playing(&mine, "the second device was let go");
    wait_until("the second output to close", || second.is_closed());
}

#[test]
fn a_sound_for_both_waits_for_the_second_device_and_starts_on_both_together() {
    let (_sound_dir, sound) = sound_file(SECOND / 2).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.factory
        .slow_device(second_device(), Duration::from_millis(300), Duration::ZERO);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(PlaySound {
        token: 3,
        path: sound,
        gain: 1.0,
        route: SoundRoute::Both,
    });
    rig.expect(SoundStarted { token: 3 });
    let mine = rig.output_on(None);
    let second = rig.output_on(Some(&second_device()));
    // Nothing of it was heard on either before the other: both start at the sound's start.
    let on_mine = mine.pull(SECOND / 10);
    let on_second = second.pull(SECOND / 10);
    assert!(audible_frames(&on_mine, 2) > 0);
    assert!(
        on_mine == on_second,
        "the two parts of the sound are not in step"
    );
}

#[test]
fn a_sound_stopped_while_the_second_device_opens_never_plays() {
    let (_sound_dir, sound) = sound_file(SECOND / 4).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.factory
        .slow_device(second_device(), Duration::from_millis(400), Duration::ZERO);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(PlaySound {
        token: 3,
        path: sound,
        gain: 1.0,
        route: SoundRoute::Second,
    });
    rig.settle();
    rig.send(StopSounds);
    let mut events = Vec::new();
    loop {
        let event = rig.next_event();
        events.push(event.clone());
        if event == (SoundEnded { token: 3 }) {
            break;
        }
    }
    let second = rig.output_on(Some(&second_device()));
    rig.settle();
    assert!(second.pull(SECOND / 10).iter().all(|sample| *sample == 0.0));
    events.extend(rig.drain_events());
    assert!(
        !events.contains(&SoundStarted { token: 3 }),
        "the stopped sound started: {events:?}"
    );
}

#[test]
fn a_copy_that_waits_for_a_slow_second_device_plays_in_step_at_once() {
    let (_dir, path) = tone(4 * SECOND as u64).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.factory
        .slow_device(second_device(), Duration::from_millis(300), Duration::ZERO);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(share(0.5));
    rig.send(Load {
        track: TrackRequest { token: 1, path },
        start_ms: 0,
        play: true,
    });
    let mine = rig.output_on(None);
    // The music plays on while the second device opens.
    while rig.factory.outputs_on(Some(&second_device())).is_empty() {
        mine.pull(SECOND / 100);
        thread::sleep(Duration::from_millis(10));
    }
    let second = rig.second_in_use();

    // What the music had queued is in the copy, not silence in its place.
    let local = mine.pull(SECOND / 4);
    let copy = second.pull(SECOND / 4);
    assert!(audible_frames(&local, 2) > 0);
    assert!(
        copy[FADE..] == scaled(&local[FADE..], 0.5)[..],
        "the copy is not in step with the music"
    );
}

/// A measurement in real time, which other tests running beside it can slow down: run it alone
/// with `cargo test -p aria-audio --test chat -- --ignored --nocapture`. On the development PC it
/// measured 38 to 48 ms; it measured 100 ms before the microphone's converter worked in short
/// chunks.
#[test]
#[ignore = "a real-time measurement; run it alone"]
fn a_16_khz_microphone_reaches_the_cable_within_60_ms() {
    // A headset microphone at 16 kHz, mono, into a cable at 44.1 kHz: resampled on the way.
    let rig = Rig::with_microphone(
        STEREO_44K,
        IDLE_GRACE,
        CaptureFormat {
            sample_rate: 16_000,
            channels: 1,
        },
    );
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(microphone(true, 1.0));
    rig.expect(MicrophoneOpened { name: None });
    let second = rig.second_in_use();

    // In real time, 10 ms at a time, as a device period: the microphone brings 10 ms, the cable
    // takes 10 ms. A burst of 2 ms is spoken at 200 ms.
    const STEPS: usize = 40;
    const BURST_STEP: usize = 20;
    let mut heard = Vec::new();
    let started = Instant::now();
    for step in 0..STEPS {
        let mut spoken = vec![0.0f32; 160];
        if step == BURST_STEP {
            spoken[..32].fill(0.8);
        }
        rig.mic.push(&spoken);
        let due = started + Duration::from_millis(10 * (step as u64 + 1));
        thread::sleep(due.saturating_duration_since(Instant::now()));
        heard.extend(second.pull(441));
    }
    let first = heard
        .as_chunks::<2>()
        .0
        .iter()
        .position(|frame| frame[0].abs() > 0.2)
        .expect("the burst never reached the cable");
    // Each step's pull came after its push, so a burst heard in the step it was spoken is 0 ms
    // late; the step it was spoken in starts at frame BURST_STEP * 441.
    let late_ms = (first as f64 - (BURST_STEP * 441) as f64) * 1_000.0 / 44_100.0;
    println!("the microphone reached the cable {late_ms:.1} ms after it was spoken");
    assert!(late_ms < 60.0, "{late_ms:.1} ms");
}
