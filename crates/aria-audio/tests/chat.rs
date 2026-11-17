//! The second output as a chat's microphone: the music copied into it, and the microphone.
//!
//! Driven through fake outputs and a fake capture, so no device and no real microphone is used.
//! Nothing plays until a test pulls frames.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use aria_audio::fake::{FakeControl, FakeFactory};
use aria_audio::testing::write_tone_wav;
use aria_audio::{
    ClockReading, DeviceId, DeviceInfo, Engine, EngineCommand, EngineEvent, EngineHandle, FADE_MS,
    IDLE_GRACE, OutputFormat, OutputPath, OutputState, SoundRoute, TrackDecoder, TrackRequest,
};
use tempfile::TempDir;

use EngineCommand::{
    Load, Pause, Play, PlaySound, Seek, SetDevice, SetExclusive, SetShareMusic, SetSoundDevice,
    SetVolume,
};
use EngineEvent::{Output, SoundEnded, SoundStarted};

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

fn assert_near(actual: usize, expected: usize) {
    let margin = expected / 50;
    assert!(
        actual.abs_diff(expected) <= margin,
        "{actual} frames, expected {expected} within 2%"
    );
}

fn scaled(samples: &[f32], gain: f32) -> Vec<f32> {
    samples.iter().map(|sample| sample * gain).collect()
}

fn share(gain: f32) -> EngineCommand {
    SetShareMusic { on: true, gain }
}

struct Rig {
    factory: FakeFactory,
    engine: EngineHandle,
    events: Receiver<EngineEvent>,
}

impl Rig {
    fn new(format: OutputFormat) -> Self {
        Self::with_idle_grace(format, IDLE_GRACE)
    }

    fn with_idle_grace(format: OutputFormat, grace: Duration) -> Self {
        let factory = FakeFactory::new(format);
        let (sender, events) = mpsc::channel();
        let on_event = move |event| {
            let _ = sender.send(event);
        };
        let engine = match Engine::start_with_idle_grace(
            Box::new(factory.clone()),
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

/// A rig sharing the music at `gain` to the second device, with `path` playing and its rings full.
fn sharing(gain: f32, path: &Path) -> (Rig, FakeControl, FakeControl) {
    let rig = Rig::new(STEREO_44K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(share(gain));
    rig.start(1, path);
    let mine = rig.output_on(None);
    let second = rig.output_on(Some(&second_device()));
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
    rig.settle();
    // Each output acts on the seek when it is next pulled, and then gets the audio from there.
    mine.pull(SECOND / 10);
    second.pull(SECOND / 100);
    rig.settle();
    let copy = second.pull(SECOND / 4);
    let from = 2 * 44_541;
    let sounding: Vec<f32> = copy
        .as_chunks::<2>()
        .0
        .iter()
        .skip_while(|frame| frame.iter().all(|sample| *sample == 0.0))
        .flatten()
        .copied()
        .collect();
    assert!(sounding.len() > SECOND / 4, "the copy did not carry on");
    let check = &sounding[FADE..SECOND / 4];
    let expected = &music[from + FADE..from + SECOND / 4];
    assert!(check == expected, "the copy did not seek with the music");
}

#[test]
fn the_copy_is_resampled_to_the_second_output() {
    let frames = SECOND / 2;
    let (_dir, path) = tone(frames as u64).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.factory.set_shared_format(second_device(), STEREO_48K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(share(1.0));
    rig.start(1, &path);
    let mine = rig.output_on(None);
    let second = rig.output_on(Some(&second_device()));
    assert_eq!(second.opened_as().1.sample_rate, 48_000);

    mine.pull(SECOND);
    rig.settle();
    let copy = second.pull(48_000);

    assert_near(audible_frames(&copy, 2), 24_000);
}

#[test]
fn a_full_copy_ring_never_stalls_the_music() {
    let (_dir, path) = tone(5 * SECOND as u64).unwrap();
    let (_rig, mine, _second) = sharing(1.0, &path);

    // The second output is never pulled, so the copy's ring fills and stays full.
    let mut heard = Vec::new();
    let deadline = Instant::now() + DEADLINE;
    while heard.len() < 4 * 2 * SECOND {
        heard.extend(mine.pull(SECOND / 10));
        thread::sleep(Duration::from_millis(2));
        assert!(Instant::now() < deadline, "the music stalled");
    }

    assert_near(audible_frames(&heard, 2), 4 * SECOND);
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
