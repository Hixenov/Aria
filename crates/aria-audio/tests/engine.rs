//! The player engine, driven through the fake output.
//!
//! Nothing plays until a test pulls frames, so no timing here depends on the clock on the wall.
//! Every wait is on a condition, with a deadline that only a failure ever reaches.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use aria_audio::fake::{FakeCapture, FakeControl, FakeFactory};
use aria_audio::testing::{write_gapless_pair, write_tone_wav};
use aria_audio::{
    AudioError, ClockReading, DeviceFormat, DeviceId, DeviceInfo, DeviceLister, Engine,
    EngineCommand, EngineEvent, EngineHandle, FADE_MS, IDLE_GRACE, Opened, OutputFactory,
    OutputFormat, OutputPath, OutputRequest, OutputState, PlaybackClock, SOUNDS_AT_ONCE,
    SampleKind, SoundRoute, TrackDecoder, TrackRequest,
};
use tempfile::TempDir;

use EngineCommand::{
    FadeOut, Load, Pause, Play, PlaySound, Seek, SetDevice, SetExclusive, SetGapless, SetMuted,
    SetNext, SetSoundDevice, SetVolume, Stop, StopSounds,
};
use EngineEvent::{
    DeviceFellBack, ExclusiveUnavailable, NoDevice, Output, SoundDeviceFailed, SoundEnded,
    SoundFailed, SoundStarted, Stopped, TrackEnded, TrackFailed, TrackStarted,
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
const CHUNK_FRAMES: usize = 2_048;
const SECOND: usize = 44_100;
/// The steepest step of the fixtures' 440 Hz tone is under 0.01 at these sample rates.
const SMOOTH_STEP: f32 = 0.02;

/// A microphone nobody speaks into. These tests never turn it on.
fn no_microphone() -> Box<FakeCapture> {
    Box::new(
        FakeCapture::new(aria_audio::CaptureFormat {
            sample_rate: 44_100,
            channels: 2,
        })
        .0,
    )
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn track(token: u64, name: &str) -> TrackRequest {
    TrackRequest {
        token,
        path: fixture(name),
        gain: 1.0,
    }
}

fn load(track: TrackRequest) -> EngineCommand {
    Load {
        track,
        start_ms: 0,
        play: true,
    }
}

fn started(token: u64) -> EngineEvent {
    TrackStarted {
        token,
        duration_ms: Some(500),
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(1));
    }
}

/// Frames in which something can be heard.
fn audible_frames(samples: &[f32]) -> usize {
    samples
        .as_chunks::<2>()
        .0
        .iter()
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

/// The samples from the first sound to the last.
fn trimmed(samples: &[f32]) -> &[f32] {
    let first = samples.iter().position(|s| *s != 0.0).unwrap_or(0);
    let last = samples.iter().rposition(|s| *s != 0.0).unwrap_or(0);
    &samples[first..=last]
}

fn longest_zero_run(samples: &[f32]) -> usize {
    samples
        .split(|sample| *sample != 0.0)
        .map(<[f32]>::len)
        .max()
        .unwrap_or(0)
}

/// The largest jump between neighbouring frames of one channel.
fn largest_step(samples: &[f32]) -> f32 {
    samples
        .iter()
        .step_by(2)
        .zip(samples.iter().step_by(2).skip(1))
        .map(|(a, b)| (b - a).abs())
        .fold(0.0, f32::max)
}

/// An engine on a fake output, with its events collected.
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
            no_microphone(),
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

    /// Output number `index`, once it has opened.
    fn output(&self, index: usize) -> FakeControl {
        wait_until("an output", || self.factory.outputs().len() > index);
        self.factory.outputs()[index].clone()
    }

    fn next_event(&self) -> EngineEvent {
        match self.events.recv_timeout(DEADLINE) {
            Ok(event) => event,
            Err(error) => panic!("no event came: {error}"),
        }
    }

    /// The next event that is not about the output's state.
    fn next_track_event(&self) -> EngineEvent {
        loop {
            match self.next_event() {
                Output(_) => {}
                event => return event,
            }
        }
    }

    /// The next state the engine reports, passing over other events.
    fn next_state(&self) -> OutputState {
        loop {
            if let Output(state) = self.next_event() {
                return state;
            }
        }
    }

    fn assert_no_event(&self) {
        let event = self.events.try_recv();
        assert!(event.is_err(), "unexpected {event:?}");
    }

    /// Pause, and return once the engine has filled the ring as far as it can.
    ///
    /// The clock is published at the end of each pass, after decoding, and commands are taken in
    /// order. So a clock that shows the pause also shows that decoding caught up with everything
    /// sent before it.
    fn hold(&self, token: u64) {
        self.send(Pause);
        wait_until("the pause", || {
            let reading = self.read();
            reading.token == token && !reading.playing
        });
    }

    /// Undo [`hold`](Self::hold), with the same promise about decoding.
    fn resume(&self) {
        self.send(Play);
        wait_until("playback", || self.read().playing);
    }

    /// While paused, return once the clock has taken in every frame pulled so far.
    fn sync(&self, token: u64) {
        self.resume();
        self.hold(token);
    }

    /// Start `track` with the ring already full, so that pulls never run out of audio.
    fn start(&self, track: TrackRequest) {
        let token = track.token;
        self.send(load(track));
        self.hold(token);
        self.resume();
    }

    /// Start `first` with `second` joined behind it in the ring.
    fn start_pair(&self, first: TrackRequest, second: TrackRequest) {
        let token = first.token;
        self.send(load(first));
        self.send(SetNext(Some(second)));
        self.hold(token);
        self.resume();
    }

    /// Pull a little and see the track start.
    fn hear_start(&self, output: &FakeControl) {
        output.pull(4_096);
        let event = self.next_track_event();
        assert!(matches!(event, TrackStarted { .. }), "got {event:?}");
    }

    /// Pull frames until an event matches, returning the audio and the events up to it.
    fn pull_until(
        &self,
        output: &FakeControl,
        last: impl Fn(&EngineEvent) -> bool,
    ) -> (Vec<f32>, Vec<EngineEvent>) {
        let deadline = Instant::now() + DEADLINE;
        let (mut audio, mut events) = (Vec::new(), Vec::new());
        loop {
            audio.extend(output.pull(CHUNK_FRAMES));
            while let Ok(event) = self.events.try_recv() {
                let done = last(&event);
                events.push(event);
                if done {
                    return (audio, events);
                }
            }
            assert!(Instant::now() < deadline, "no such event in {events:?}");
            thread::sleep(Duration::from_micros(200));
        }
    }

    /// Pull exactly `frames` frames and wait until the engine's clock has seen them.
    fn pull_heard(&self, output: &FakeControl, frames: usize, position_ms: u64) -> Vec<f32> {
        let audio = output.pull(frames);
        wait_until("the clock to move", || {
            self.read().position_ms >= position_ms
        });
        audio
    }

    fn pull_to_stop(&self, output: &FakeControl) -> (Vec<f32>, Vec<EngineEvent>) {
        self.pull_until(output, |event| *event == Stopped)
    }
}

/// A WAV in a temporary folder that goes when the guard does.
fn tone_file(frames: u64) -> io::Result<(TempDir, PathBuf)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tone.wav");
    write_tone_wav(&path, 44_100, 2, frames, 440.0)?;
    Ok((dir, path))
}

/// A copy of `tone.m4a` with its audio overwritten by `fill` after the first `keep_percent` of it.
/// The index that may follow the audio stays, so the file still opens.
fn damaged_m4a(dir: &Path, keep_percent: usize, fill: u8) -> io::Result<PathBuf> {
    let mut bytes = fs::read(fixture("tone.m4a"))?;
    let name_at = bytes
        .windows(4)
        .position(|name| name == b"mdat")
        .unwrap_or(4);
    let size = bytes[name_at - 4..name_at]
        .iter()
        .fold(0, |size, byte| size << 8 | usize::from(*byte));
    let audio = name_at + 4;
    let audio_end = name_at - 4 + size;
    bytes[audio + (audio_end - audio) * keep_percent / 100..audio_end].fill(fill);
    let path = dir.join("damaged.m4a");
    fs::write(&path, bytes)?;
    Ok(path)
}

#[test]
fn plays_a_track_to_the_end() {
    let rig = Rig::new(STEREO_44K);

    rig.send(load(track(1, "tone.flac")));
    let (audio, events) = rig.pull_to_stop(&rig.output(0));

    let length = TrackStarted {
        token: 1,
        duration_ms: Some(1_000),
    };
    assert_eq!(events, [length, TrackEnded { token: 1 }, Stopped]);
    assert_near(audible_frames(&audio), SECOND);
    assert!(!rig.read().playing);
    assert!(rig.engine.shutdown(DEADLINE));
}

#[test]
fn plays_aiff_and_caf_to_the_end() {
    for name in ["tone.aiff", "tone.caf"] {
        let rig = Rig::new(STEREO_44K);

        rig.send(load(track(1, name)));
        let (audio, events) = rig.pull_to_stop(&rig.output(0));

        assert_eq!(events.last(), Some(&Stopped), "{name}: {events:?}");
        assert!(
            events.contains(&TrackEnded { token: 1 }),
            "{name}: {events:?}"
        );
        assert_near(audible_frames(&audio), SECOND);
        assert!(rig.engine.shutdown(DEADLINE));
    }
}

/// Long enough for a pause to settle, short enough to wait for.
const SHORT_GRACE: Duration = Duration::from_millis(300);

#[test]
fn a_paused_output_closes_after_the_grace_and_play_reopens_it_in_place() {
    let rig = Rig::with_idle_grace(STEREO_44K, SHORT_GRACE);
    rig.start(track(1, "tone.flac"));
    let first = rig.output(0);
    rig.pull_heard(&first, 22_050, 400);
    rig.hold(1);
    let paused_at = rig.read().position_ms;

    wait_until("the output to close", || first.is_closed());
    assert!(
        rig.read().position_ms.abs_diff(paused_at) <= 2,
        "closing moved the clock"
    );
    assert!(!rig.read().playing);
    assert_eq!(rig.factory.outputs().len(), 1, "nothing reopened by itself");

    rig.resume();
    let second = rig.output(1);
    assert!(!second.is_closed());
    assert!(rig.read().position_ms.abs_diff(paused_at) <= 5);
    rig.pull_heard(&second, 4_410, paused_at + 90);
    assert!(rig.read().position_ms >= paused_at + 90);
    assert!(rig.engine.shutdown(DEADLINE));
}

#[test]
fn a_pause_shorter_than_the_grace_keeps_the_output() {
    let rig = Rig::with_idle_grace(STEREO_44K, Duration::from_secs(60));
    rig.start(track(1, "tone.flac"));
    let first = rig.output(0);
    rig.hold(1);
    thread::sleep(Duration::from_millis(300));
    rig.resume();

    assert!(!first.is_closed());
    assert_eq!(rig.factory.outputs().len(), 1);
    assert!(rig.engine.shutdown(DEADLINE));
}

#[test]
fn an_output_with_nothing_loaded_closes_after_the_grace() {
    let rig = Rig::with_idle_grace(STEREO_44K, SHORT_GRACE);
    rig.start(track(1, "tone.flac"));
    let first = rig.output(0);
    rig.send(Stop);

    wait_until("the output to close", || first.is_closed());
    assert!(rig.engine.shutdown(DEADLINE));
}

#[test]
fn gapless_join_has_no_gap() {
    let rig = Rig::new(STEREO_44K);

    rig.start_pair(
        track(1, "silence-gap-a.flac"),
        track(2, "silence-gap-b.flac"),
    );
    let audio = rig.output(0).pull(SECOND + 1_000);

    let heard = trimmed(&audio);
    assert_near(heard.len() / 2, SECOND);
    assert!(longest_zero_run(heard) <= 2, "a gap at the join");
    assert!(largest_step(heard) < SMOOTH_STEP, "the waveform jumped");
}

#[test]
fn gapless_join_survives_resampling() {
    // 11,050 frames puts the join near the top of the wave, where an edge in the filter would show.
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = (dir.path().join("a.wav"), dir.path().join("b.wav"));
    write_gapless_pair(&first, &second, 44_100, 440.0, 11_050).unwrap();
    let rig = Rig::new(STEREO_48K);
    let request = |token, path: &PathBuf| TrackRequest {
        token,
        path: path.clone(),
        gain: 1.0,
    };

    rig.start_pair(request(1, &first), request(2, &second));
    let audio = rig.output(0).pull(25_000);

    let heard = trimmed(&audio);
    assert_near(heard.len() / 2, 24_054);
    assert!(longest_zero_run(heard) <= 2, "a gap at the join");
    // The very ends of the stream are the filter's real edges, and the fade in.
    let inner = &heard[2_000..heard.len() - 2_000];
    let step = largest_step(inner);
    assert!(step < 0.035, "the waveform jumped by {step}");
}

#[test]
fn gapless_off_still_advances() {
    let rig = Rig::new(STEREO_44K);
    rig.send(SetGapless(false));

    rig.send(load(track(1, "silence-gap-a.flac")));
    rig.send(SetNext(Some(track(2, "silence-gap-b.flac"))));
    let (audio, events) = rig.pull_to_stop(&rig.output(0));

    assert_eq!(
        events,
        [
            started(1),
            TrackEnded { token: 1 },
            started(2),
            TrackEnded { token: 2 },
            Stopped,
        ]
    );
    assert_near(audible_frames(&audio), SECOND);
}

#[test]
fn rate_change_between_tracks() {
    let rig = Rig::new(STEREO_44K);

    rig.send(load(track(1, "tone.flac")));
    rig.send(SetNext(Some(track(2, "tone-48k.flac"))));
    let (audio, events) = rig.pull_to_stop(&rig.output(0));

    let ended: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, TrackEnded { .. }))
        .collect();
    assert_eq!(ended, [&TrackEnded { token: 1 }, &TrackEnded { token: 2 }]);
    // The first track passes through untouched, so the rest is the second.
    assert_near(audible_frames(&audio) - SECOND, SECOND);
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn started_fires_when_heard() {
    let rig = Rig::new(STEREO_44K);
    rig.start_pair(
        track(1, "silence-gap-a.flac"),
        track(2, "silence-gap-b.flac"),
    );
    rig.assert_no_event();
    let output = rig.output(0);

    output.pull(1_000);
    assert_eq!(rig.next_event(), started(1));

    // 22,050 frames is the join. Stop 200 frames short of it.
    rig.pull_heard(&output, 20_850, 490);
    assert_eq!(rig.read().token, 1);
    rig.assert_no_event();

    output.pull(400);
    assert_eq!(rig.next_event(), TrackEnded { token: 1 });
    assert_eq!(rig.next_event(), started(2));
    wait_until("the clock to change tracks", || rig.read().token == 2);
}

#[test]
fn short_tracks_keep_their_order() {
    let dir = tempfile::tempdir().unwrap();
    let clip = |token: u64| {
        let path = dir.path().join(format!("{token}.wav"));
        write_tone_wav(&path, 44_100, 2, 4_410, 440.0).unwrap();
        TrackRequest {
            token,
            path,
            gain: 1.0,
        }
    };
    let short = |token| TrackStarted {
        token,
        duration_ms: Some(100),
    };
    let rig = Rig::new(STEREO_44K);
    rig.start_pair(clip(1), clip(2));
    let output = rig.output(0);

    // Just past the join: the first clip is over, and both were in the ring at once.
    output.pull(4_410 + 64);
    assert_eq!(rig.next_event(), short(1));
    assert_eq!(rig.next_event(), TrackEnded { token: 1 });
    assert_eq!(rig.next_event(), short(2));

    // The backend announces each next track once the one before it starts.
    rig.send(SetNext(Some(clip(3))));
    rig.hold(2);
    rig.resume();
    let mut events = Vec::new();
    let deadline = Instant::now() + DEADLINE;
    while events.last() != Some(&Stopped) {
        assert!(Instant::now() < deadline, "only {events:?} arrived");
        output.pull(64);
        events.extend(rig.events.try_iter());
        let reading = rig.read();
        assert!(reading.position_ms <= 100, "the clock said {reading:?}");
    }

    assert_eq!(
        events,
        [
            TrackEnded { token: 2 },
            short(3),
            TrackEnded { token: 3 },
            Stopped,
        ]
    );
}

#[test]
fn next_track_vanishing_is_reported() {
    // Longer than the ring, so the join is still ahead when the file goes.
    let (_dir, long) = tone_file(3 * SECOND as u64).unwrap();
    let (gone_dir, gone) = tone_file(SECOND as u64).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.send(load(TrackRequest {
        token: 1,
        path: long,
        gain: 1.0,
    }));
    rig.send(SetNext(Some(TrackRequest {
        token: 2,
        path: gone.clone(),
        gain: 1.0,
    })));
    rig.hold(1);
    fs::remove_file(&gone).unwrap();
    drop(gone_dir);
    rig.resume();
    let output = rig.output(0);

    let (_, events) = rig.pull_until(&output, |event| *event == TrackFailed { token: 2 });

    let duration = Some(3_000);
    assert_eq!(
        events,
        [
            TrackStarted {
                token: 1,
                duration_ms: duration
            },
            TrackEnded { token: 1 },
            TrackFailed { token: 2 },
        ]
    );
    assert!(!rig.read().playing);

    // Nothing advanced by itself, and the thread still answers.
    rig.send(load(track(3, "tone.flac")));
    let (_, events) = rig.pull_to_stop(&output);
    assert_eq!(events.last(), Some(&Stopped));
    assert!(events.contains(&TrackEnded { token: 3 }));
}

#[test]
fn next_track_vanishing_without_gapless_is_reported() {
    let rig = Rig::new(STEREO_44K);
    rig.send(SetGapless(false));
    rig.send(load(track(1, "silence-gap-a.flac")));
    rig.send(SetNext(Some(track(2, "no-such-file.flac"))));
    let output = rig.output(0);

    let (_, events) = rig.pull_until(&output, |event| *event == TrackFailed { token: 2 });

    assert_eq!(
        events,
        [
            started(1),
            TrackEnded { token: 1 },
            TrackFailed { token: 2 }
        ]
    );
}

#[test]
fn missing_file_fails_load() {
    let rig = Rig::new(STEREO_44K);

    rig.send(load(track(1, "no-such-file.flac")));

    assert_eq!(rig.next_event(), TrackFailed { token: 1 });
    rig.send(Play);
    rig.send(load(track(2, "no-such-file.flac")));
    assert_eq!(rig.next_event(), TrackFailed { token: 2 });
    assert!(rig.factory.outputs().is_empty());
}

#[test]
fn a_file_that_is_not_audio_fails_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lying.flac");
    fs::write(&path, "not audio at all").unwrap();
    let rig = Rig::new(STEREO_44K);

    rig.send(load(TrackRequest {
        token: 1,
        path,
        gain: 1.0,
    }));

    assert_eq!(rig.next_event(), TrackFailed { token: 1 });
}

#[test]
fn a_track_that_breaks_plays_what_it_can_then_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = damaged_m4a(dir.path(), 50, 0xFF).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.send(load(TrackRequest {
        token: 1,
        path,
        gain: 1.0,
    }));
    rig.send(SetNext(Some(track(2, "tone.flac"))));

    let output = rig.output(0);
    let (audio, events) = rig.pull_until(&output, |event| *event == TrackFailed { token: 1 });

    assert!(matches!(events[0], TrackStarted { token: 1, .. }));
    assert_eq!(events.len(), 2, "got {events:?}");
    assert!(audible_frames(&audio) > SECOND / 4);
    assert!(audible_frames(&audio) < SECOND * 3 / 4);
    wait_until("the pause", || !rig.read().playing);
}

#[test]
fn a_next_track_that_breaks_after_a_resampled_join_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = damaged_m4a(dir.path(), 0, 0).unwrap();
    let rig = Rig::new(STEREO_48K);

    rig.send(load(track(1, "tone.flac")));
    rig.send(SetNext(Some(TrackRequest {
        token: 2,
        path,
        gain: 1.0,
    })));
    let output = rig.output(0);
    let (audio, events) = rig.pull_until(&output, |event| *event == TrackFailed { token: 2 });

    let whole = TrackStarted {
        token: 1,
        duration_ms: Some(1_000),
    };
    assert_eq!(
        events,
        [whole, TrackEnded { token: 1 }, TrackFailed { token: 2 }]
    );
    // All of the first track, with the part the resampler was still holding.
    let heard = audible_frames(&audio);
    assert!(heard > 47_900, "only {heard} frames were heard");
    wait_until("the pause", || !rig.read().playing);
}

#[test]
fn a_next_track_set_after_a_track_breaks_waits_for_a_command() {
    let dir = tempfile::tempdir().unwrap();
    let path = damaged_m4a(dir.path(), 50, 0xFF).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.start(TrackRequest {
        token: 1,
        path,
        gain: 1.0,
    });
    let output = rig.output(0);

    // The failure is known by now, but the audio before it has not been heard.
    rig.hold(1);
    rig.send(SetNext(Some(track(2, "tone.flac"))));
    rig.resume();
    let (_, events) = rig.pull_until(&output, |event| *event == TrackFailed { token: 1 });

    assert_eq!(events.len(), 2, "got {events:?}");
    wait_until("the pause", || !rig.read().playing);
    assert_eq!(rig.read().token, 1);
    assert!(output.pull(4_096).iter().all(|sample| *sample == 0.0));
    rig.send(load(track(3, "no-such-file.flac")));
    assert_eq!(rig.next_event(), TrackFailed { token: 3 });
}

#[test]
fn loading_replaces_the_playing_track() {
    let rig = Rig::new(STEREO_44K);
    rig.start_pair(track(1, "tone.flac"), track(2, "tone.wav"));
    let output = rig.output(0);
    rig.hear_start(&output);

    rig.send(load(track(3, "silence-gap-a.flac")));
    let (_, events) = rig.pull_to_stop(&output);

    // The old track is gone without a word, and the old next track went with it.
    assert_eq!(events, [started(3), TrackEnded { token: 3 }, Stopped]);
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn impatient_seeks_settle_on_the_last() {
    let rig = Rig::new(STEREO_44K);
    rig.send(load(track(1, "tone.flac")));
    rig.hold(1);
    let output = rig.output(0);

    for position in 0..10 {
        rig.send(Seek(position * 40));
    }
    rig.send(Seek(500));

    wait_until("the last seek", || {
        rig.read().position_ms.abs_diff(500) <= 50
    });
    assert!(!rig.read().playing);
    output.pull(4_096);
    rig.sync(1);
    assert!(rig.read().position_ms.abs_diff(500) <= 50, "it drifted");

    rig.send(Seek(10_000_000));
    let (_, events) = rig.pull_to_stop(&output);

    assert_eq!(
        events[events.len() - 2..],
        [TrackEnded { token: 1 }, Stopped]
    );
    let reading = rig.read();
    assert!(reading.position_ms <= 1_000, "the clock said {reading:?}");
    assert!(reading.position_ms >= 990, "the clock said {reading:?}");
}

#[test]
fn seek_while_paused_plays_from_there() {
    let rig = Rig::new(STEREO_44K);
    rig.send(load(track(1, "tone.flac")));
    rig.hold(1);
    let output = rig.output(0);

    rig.send(Seek(750));
    wait_until("the seek", || rig.read().position_ms == 750);
    assert!(output.pull(4_096).iter().all(|sample| *sample == 0.0));
    assert!(!rig.read().playing);
    rig.resume();
    let (audio, _) = rig.pull_to_stop(&output);

    assert_near(audible_frames(&audio), SECOND / 4);
}

#[test]
fn seek_past_the_end_moves_to_the_next_track() {
    let rig = Rig::new(STEREO_44K);
    rig.start_pair(
        track(1, "silence-gap-a.flac"),
        track(2, "silence-gap-b.flac"),
    );
    let output = rig.output(0);
    rig.hear_start(&output);

    rig.send(Seek(10_000_000));
    let (_, events) = rig.pull_to_stop(&output);

    assert_eq!(
        events,
        [
            TrackEnded { token: 1 },
            started(2),
            TrackEnded { token: 2 },
            Stopped,
        ]
    );
}

#[test]
fn seek_reaches_back_past_the_track_in_the_ring() {
    let rig = Rig::new(STEREO_44K);
    rig.start_pair(
        track(1, "silence-gap-a.flac"),
        track(2, "silence-gap-b.flac"),
    );
    let output = rig.output(0);
    rig.pull_heard(&output, 11_025, 240);

    // The decoder has moved on to the second track. The first is the one being heard.
    rig.send(Seek(100));
    wait_until("the seek", || {
        let reading = rig.read();
        reading.token == 1 && reading.position_ms < 200
    });
    let (audio, events) = rig.pull_to_stop(&output);

    assert_eq!(
        events,
        [
            started(1),
            TrackEnded { token: 1 },
            started(2),
            TrackEnded { token: 2 },
            Stopped,
        ]
    );
    assert_near(audible_frames(&audio), SECOND * 9 / 10);
}

#[test]
fn a_seek_while_playing_does_not_break_off_once_the_new_audio_starts() {
    let rig = Rig::new(STEREO_44K);
    let (_dir, path) = match tone_file(SECOND as u64 * 4) {
        Ok(file) => file,
        Err(error) => panic!("no tone file: {error}"),
    };
    rig.start(TrackRequest {
        token: 1,
        path,
        gain: 1.0,
    });
    let output = rig.output(0);
    // While playing, the ring has room when a seek comes.
    output.pull(CHUNK_FRAMES);
    rig.send(Seek(2_000));
    wait_until("the seek", || rig.read().position_ms >= 1_900);

    // The old audio fades out first. The new audio may then wait, as long as a slow machine
    // needs, but once it starts it must not stop: a stop is a cut to silence with no fade, and
    // it is heard as a click.
    let fade = SECOND * FADE_MS as usize / 1_000;
    let mut audio = output.pull(CHUNK_FRAMES * 2);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !audio.iter().skip(fade * 2).any(|sample| *sample != 0.0) {
        assert!(
            std::time::Instant::now() < deadline,
            "the new audio never came"
        );
        thread::sleep(Duration::from_millis(20));
        audio.extend(output.pull(CHUNK_FRAMES / 4));
    }
    // Once it has started, the decoder has had time to fill the ring: what follows is whole.
    thread::sleep(Duration::from_millis(100));
    let after = output.pull(CHUNK_FRAMES * 4);
    let gap = longest_zero_run(&after);
    assert!(gap <= 2, "the new audio broke off for {} frames", gap / 2);
}

#[test]
fn pause_stops_consumption() {
    let rig = Rig::new(STEREO_44K);
    rig.start(track(1, "tone.flac"));
    let output = rig.output(0);
    rig.pull_heard(&output, 11_025, 240);

    rig.hold(1);
    // The fade out still takes a few frames.
    output.pull(1_024);
    rig.sync(1);
    let held = rig.read().position_ms;
    assert!(output.pull(4_096).iter().all(|sample| *sample == 0.0));
    rig.sync(1);
    assert_eq!(rig.read().position_ms, held);

    rig.resume();
    rig.pull_heard(&output, 4_410, held + 90);
    assert!(rig.read().playing);
}

#[test]
fn volume_reaches_the_output_opened_later() {
    let rig = Rig::new(STEREO_44K);
    rig.send(SetVolume(0.0));

    rig.start(track(1, "tone.flac"));
    let silent = rig.output(0).pull(8_192);
    assert!(silent.iter().all(|sample| *sample == 0.0));

    rig.send(SetVolume(1.0));
    rig.sync(1);
    rig.resume();
    let (audio, _) = rig.pull_to_stop(&rig.output(0));
    assert!(audible_frames(&audio) > 0);
}

#[test]
fn device_loss_resumes_at_position() {
    let rig = Rig::new(STEREO_44K);
    let chosen = DeviceId("chosen".into());
    rig.send(SetDevice(Some(chosen.clone())));
    rig.start(track(1, "tone.flac"));
    let first = rig.output(0);
    assert_eq!(first.device(), Some(chosen));
    rig.pull_heard(&first, SECOND / 2, 490);
    assert!(matches!(rig.next_event(), TrackStarted { .. }));
    let before = rig.read();

    // The chosen device is gone, so only the default answers.
    rig.factory.fail_open(1);
    first.fail();

    assert_eq!(rig.next_event(), DeviceFellBack);
    let second = rig.output(1);
    assert_eq!(second.device(), None);
    assert!(rig.read().playing);
    rig.hold(1);
    let after = rig.read();
    assert_eq!(after.token, 1);
    assert!(
        after.position_ms.abs_diff(before.position_ms) <= 100,
        "{before:?} became {after:?}"
    );
    rig.resume();

    let (audio, events) = rig.pull_to_stop(&second);
    assert_eq!(events, [TrackEnded { token: 1 }, Stopped]);
    assert_near(audible_frames(&audio), SECOND / 2);
    assert_eq!(rig.factory.outputs().len(), 2);
}

#[test]
fn device_loss_on_the_system_default_shows_a_notice() {
    let rig = Rig::new(STEREO_44K);
    rig.start(track(1, "tone.flac"));
    let first = rig.output(0);
    rig.hear_start(&first);

    first.fail();

    assert_eq!(rig.next_event(), DeviceFellBack);
    assert_eq!(rig.output(1).device(), None);
}

#[test]
fn device_loss_brings_back_the_next_track_in_the_ring() {
    let rig = Rig::new(STEREO_44K);
    rig.start_pair(
        track(1, "silence-gap-a.flac"),
        track(2, "silence-gap-b.flac"),
    );
    let first = rig.output(0);
    rig.pull_heard(&first, 11_025, 240);

    first.fail();
    let second = rig.output(1);
    let (audio, events) = rig.pull_to_stop(&second);

    assert_eq!(
        events,
        [
            started(1),
            DeviceFellBack,
            TrackEnded { token: 1 },
            started(2),
            TrackEnded { token: 2 },
            Stopped,
        ]
    );
    assert_near(audible_frames(&audio), SECOND * 3 / 4);
}

#[test]
fn device_loss_while_paused_stays_paused() {
    let rig = Rig::new(STEREO_44K);
    rig.send(load(track(1, "tone.flac")));
    rig.hold(1);
    let first = rig.output(0);

    first.fail();

    assert_eq!(rig.next_event(), DeviceFellBack);
    let second = rig.output(1);
    assert!(second.pull(1_024).iter().all(|sample| *sample == 0.0));
    assert!(!rig.read().playing);
}

#[test]
fn no_device_pauses() {
    let rig = Rig::new(STEREO_44K);
    rig.send(SetDevice(Some(DeviceId("chosen".into()))));
    rig.start(track(1, "tone.flac"));
    let first = rig.output(0);
    rig.pull_heard(&first, SECOND / 2, 490);
    assert!(matches!(rig.next_event(), TrackStarted { .. }));
    let before = rig.read();

    rig.factory.fail_open(2);
    first.fail();

    assert_eq!(rig.next_event(), NoDevice);
    wait_until("the pause", || !rig.read().playing);
    assert!(rig.read().position_ms.abs_diff(before.position_ms) <= 100);
    assert_eq!(rig.factory.outputs().len(), 1);

    rig.send(Play);
    let second = rig.output(1);
    let (audio, events) = rig.pull_to_stop(&second);

    assert_eq!(events, [TrackEnded { token: 1 }, Stopped]);
    assert_near(audible_frames(&audio), SECOND / 2);
}

#[test]
fn play_with_no_device_reports_it_and_keeps_the_track() {
    let rig = Rig::new(STEREO_44K);
    rig.factory.fail_open(1);

    rig.send(load(track(1, "tone.flac")));

    assert_eq!(rig.next_event(), NoDevice);
    assert!(!rig.read().playing);

    rig.send(Play);
    let (_, events) = rig.pull_to_stop(&rig.output(0));
    assert_eq!(events.last(), Some(&Stopped));
}

#[test]
fn a_chosen_device_missing_on_the_first_open_is_quiet() {
    let rig = Rig::new(STEREO_44K);
    rig.send(SetDevice(Some(DeviceId("chosen".into()))));
    rig.factory.fail_open(1);

    rig.start(track(1, "tone.flac"));

    assert_eq!(rig.output(0).device(), None);
    rig.assert_no_event();
}

#[test]
fn a_retry_that_lands_on_the_default_is_quiet() {
    let rig = Rig::new(STEREO_44K);
    rig.send(SetDevice(Some(DeviceId("chosen".into()))));
    rig.factory.fail_open(2);
    rig.send(load(track(1, "tone.flac")));
    assert_eq!(rig.next_event(), NoDevice);

    rig.factory.fail_open(1);
    rig.resume();

    assert_eq!(rig.output(0).device(), None);
    rig.assert_no_event();
}

#[test]
fn repeated_failures_stop_rebuilding() {
    let rig = Rig::new(STEREO_44K);
    rig.send(load(track(1, "tone.flac")));

    for rebuild in 0..3 {
        rig.output(rebuild).fail();
        assert_eq!(rig.next_event(), DeviceFellBack);
        rig.output(rebuild + 1);
    }
    rig.output(3).fail();

    assert_eq!(rig.next_event(), NoDevice);
    assert_eq!(rig.factory.outputs().len(), 4, "it kept on rebuilding");
    wait_until("the pause", || !rig.read().playing);

    // Pressing play is a fresh start, and the limit counts from there.
    rig.send(Play);
    rig.output(4).fail();
    assert_eq!(rig.next_event(), DeviceFellBack);
    rig.output(5);
}

#[test]
fn load_without_play_opens_no_output() {
    let rig = Rig::new(STEREO_44K);

    rig.send(Load {
        track: track(1, "tone.flac"),
        start_ms: 250,
        play: false,
    });
    rig.hold(1);

    assert!(rig.factory.outputs().is_empty());
    assert_eq!(rig.read().position_ms, 250);

    rig.send(Seek(500));
    wait_until("the seek", || rig.read().position_ms == 500);
    assert!(rig.factory.outputs().is_empty());
    rig.assert_no_event();

    rig.send(Play);
    let (audio, _) = rig.pull_to_stop(&rig.output(0));
    assert_near(audible_frames(&audio), SECOND / 2);
}

#[test]
fn seek_past_the_end_with_no_output_ends_the_track() {
    let rig = Rig::new(STEREO_44K);
    rig.send(Load {
        track: track(1, "tone.flac"),
        start_ms: 0,
        play: false,
    });

    rig.send(Seek(10_000_000));

    let mut last = rig.next_event();
    while last != Stopped {
        last = rig.next_event();
    }
    assert!(rig.factory.outputs().is_empty());
}

#[test]
fn set_device_reopens_at_the_position_without_a_notice() {
    let rig = Rig::new(STEREO_44K);
    rig.start(track(1, "tone.flac"));
    let first = rig.output(0);
    rig.pull_heard(&first, SECOND / 4, 240);
    assert!(matches!(rig.next_event(), TrackStarted { .. }));
    let chosen = DeviceId("chosen".into());

    rig.send(SetDevice(Some(chosen.clone())));
    let second = rig.output(1);
    assert_eq!(second.device(), Some(chosen));
    assert!(rig.read().playing);

    rig.hold(1);
    assert!(rig.read().position_ms.abs_diff(250) <= 100);
    rig.send(SetDevice(None));
    let third = rig.output(2);
    assert_eq!(third.device(), None);
    rig.sync(1);
    assert!(rig.read().position_ms.abs_diff(250) <= 100);
    assert!(third.pull(1_024).iter().all(|sample| *sample == 0.0));
    rig.resume();

    let (_, events) = rig.pull_to_stop(&third);
    assert_eq!(events, [TrackEnded { token: 1 }, Stopped]);
}

#[test]
fn set_device_before_any_output_waits_for_play() {
    let rig = Rig::new(STEREO_44K);
    let chosen = DeviceId("chosen".into());

    rig.send(SetDevice(Some(chosen.clone())));
    rig.send(Load {
        track: track(1, "tone.flac"),
        start_ms: 0,
        play: false,
    });
    rig.hold(1);
    assert!(rig.factory.outputs().is_empty());

    rig.send(Play);
    assert_eq!(rig.output(0).device(), Some(chosen));
}

#[test]
fn set_device_that_is_missing_falls_back() {
    let rig = Rig::new(STEREO_44K);
    rig.send(load(track(1, "tone.flac")));
    rig.output(0);

    rig.factory.fail_open(1);
    rig.send(SetDevice(Some(DeviceId("gone".into()))));

    assert_eq!(rig.next_event(), DeviceFellBack);
    assert_eq!(rig.output(1).device(), None);
}

#[test]
fn replacing_the_next_track_applies_even_after_it_was_queued() {
    let rig = Rig::new(STEREO_44K);
    rig.start_pair(track(1, "silence-gap-a.flac"), track(2, "tone.flac"));
    let output = rig.output(0);
    rig.pull_heard(&output, 11_025, 240);

    rig.send(SetNext(Some(track(3, "silence-gap-b.flac"))));
    rig.hold(1);
    rig.resume();
    let (_, events) = rig.pull_to_stop(&output);

    assert_eq!(
        events,
        [
            started(1),
            TrackEnded { token: 1 },
            started(3),
            TrackEnded { token: 3 },
            Stopped,
        ]
    );
}

#[test]
fn clearing_the_next_track_applies_even_after_it_was_queued() {
    let rig = Rig::new(STEREO_44K);
    rig.start_pair(
        track(1, "silence-gap-a.flac"),
        track(2, "silence-gap-b.flac"),
    );
    let output = rig.output(0);
    rig.pull_heard(&output, 11_025, 240);

    rig.send(SetNext(None));
    rig.hold(1);
    rig.resume();
    let (_, events) = rig.pull_to_stop(&output);

    assert_eq!(events, [started(1), TrackEnded { token: 1 }, Stopped]);
}

#[test]
fn announcing_the_queued_track_again_leaves_the_audio_alone() {
    let rig = Rig::new(STEREO_44K);
    rig.start_pair(
        track(1, "silence-gap-a.flac"),
        track(2, "silence-gap-b.flac"),
    );
    let output = rig.output(0);

    // The same file under a new token, as the backend sends after a library change.
    rig.send(SetNext(Some(track(3, "silence-gap-b.flac"))));
    rig.hold(1);
    rig.resume();
    let audio = output.pull(SECOND + 1_000);
    let (_, events) = rig.pull_to_stop(&output);

    let heard = trimmed(&audio);
    assert_near(heard.len() / 2, SECOND);
    assert!(longest_zero_run(heard) <= 2, "a gap at the join");
    assert_eq!(
        events,
        [
            started(1),
            TrackEnded { token: 1 },
            started(3),
            TrackEnded { token: 3 },
            Stopped,
        ]
    );
}

#[test]
fn next_track_set_while_draining_still_joins() {
    let rig = Rig::new(STEREO_44K);
    rig.send(load(track(1, "silence-gap-a.flac")));
    rig.hold(1);
    rig.send(SetNext(Some(track(2, "silence-gap-b.flac"))));
    rig.hold(1);
    rig.resume();

    let audio = rig.output(0).pull(SECOND + 1_000);

    let heard = trimmed(&audio);
    assert_near(heard.len() / 2, SECOND);
    assert!(longest_zero_run(heard) <= 2, "a gap at the join");
}

#[test]
fn a_next_track_set_after_the_end_does_not_start_by_itself() {
    let rig = Rig::new(STEREO_44K);
    rig.send(load(track(1, "silence-gap-a.flac")));
    let output = rig.output(0);
    rig.pull_to_stop(&output);

    rig.send(SetNext(Some(track(2, "silence-gap-b.flac"))));
    // Only so that the next command does not arrive in the same batch and replace this one.
    thread::sleep(Duration::from_millis(50));
    rig.send(load(track(3, "no-such-file.flac")));

    assert_eq!(rig.next_event(), TrackFailed { token: 3 });
    assert_eq!(rig.read().token, 1);
    assert!(output.pull(4_096).iter().all(|sample| *sample == 0.0));
}

#[test]
fn stop_clears_the_track_and_reports_it() {
    let rig = Rig::new(STEREO_44K);
    rig.start_pair(track(1, "tone.flac"), track(2, "tone.wav"));
    let output = rig.output(0);
    rig.hear_start(&output);

    rig.send(Stop);

    assert_eq!(rig.next_event(), Stopped);
    wait_until("the clock to reset", || {
        rig.read() == ClockReading::default()
    });
    rig.send(Play);
    output.pull(4_096);
    rig.send(load(track(3, "no-such-file.flac")));
    assert_eq!(rig.next_event(), TrackFailed { token: 3 });
}

#[test]
fn shutdown_stops_the_thread() {
    let rig = Rig::new(STEREO_44K);
    rig.send(load(track(1, "tone.flac")));
    rig.output(0).pull(4_096);

    let start = Instant::now();
    assert!(rig.engine.shutdown(Duration::from_secs(2)));
    assert!(start.elapsed() < Duration::from_secs(2));
}

/// Opens nothing: the call sticks until the test lets go, like a wedged driver.
struct StuckFactory {
    reached: Sender<()>,
    release: Mutex<Receiver<()>>,
}

impl OutputFactory for StuckFactory {
    fn device_lister(&self) -> DeviceLister {
        Arc::new(|| Ok(Vec::new()))
    }

    fn open(
        &self,
        _device: Option<&DeviceId>,
        _request: OutputRequest,
    ) -> Result<Opened, AudioError> {
        let _ = self.reached.send(());
        let release = self
            .release
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = release.recv();
        Err(AudioError::NoDevice)
    }
}

#[test]
fn shutdown_gives_up_on_a_stuck_thread() {
    let (reached_tx, reached) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let factory = StuckFactory {
        reached: reached_tx,
        release: Mutex::new(release_rx),
    };
    let engine = Engine::start(Box::new(factory), no_microphone(), Box::new(|_| {})).unwrap();
    engine.send(load(track(1, "tone.flac")));
    reached.recv_timeout(DEADLINE).unwrap();

    let start = Instant::now();
    assert!(!engine.shutdown(Duration::from_millis(100)));
    assert!(start.elapsed() < Duration::from_secs(5));

    drop(release);
}

#[test]
fn shutdown_returns_promptly_after_a_panic() {
    let factory = FakeFactory::new(STEREO_44K);
    let engine = Engine::start(
        Box::new(factory),
        no_microphone(),
        Box::new(|_| panic!("test event handler panics")),
    )
    .unwrap();
    engine.send(load(track(1, "no-such-file.flac")));

    let start = Instant::now();
    assert!(!engine.shutdown(Duration::from_secs(10)));
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_panicked_engine_leaves_the_clock_stopped() {
    let factory = FakeFactory::new(STEREO_44K);
    let engine = Engine::start(
        Box::new(factory.clone()),
        no_microphone(),
        Box::new(|_| panic!("test event handler panics")),
    )
    .unwrap();
    let clock = engine.clock();
    engine.send(load(track(1, "tone.flac")));
    wait_until("playback", || clock.read().playing);

    // The first track to be heard sets off the panic.
    let output = factory.outputs()[0].clone();
    wait_until("the clock to stop", || {
        output.pull(1_024);
        !clock.read().playing
    });

    assert!(!engine.shutdown(DEADLINE));
}

#[test]
fn the_clock_is_current_when_an_event_arrives() {
    let factory = FakeFactory::new(STEREO_44K);
    let clock_slot = Arc::new(OnceLock::<Arc<PlaybackClock>>::new());
    let (sender, seen) = mpsc::channel();
    let handler_slot = Arc::clone(&clock_slot);
    let on_event = move |event| {
        let reading = handler_slot.get().map(|clock| clock.read());
        let _ = sender.send((event, reading));
    };
    let engine = Engine::start(
        Box::new(factory.clone()),
        no_microphone(),
        Box::new(on_event),
    )
    .unwrap();
    clock_slot.set(engine.clock()).ok().unwrap();

    engine.send(load(track(1, "silence-gap-a.flac")));
    wait_until("an output", || !factory.outputs().is_empty());
    let output = factory.outputs()[0].clone();
    wait_until("the end", || {
        output.pull(CHUNK_FRAMES);
        match seen.try_recv() {
            Ok((Stopped, reading)) => {
                let reading = reading.unwrap();
                assert!(!reading.playing, "the clock said {reading:?}");
                assert!(reading.position_ms >= 490, "the clock said {reading:?}");
                true
            }
            Ok((TrackStarted { .. }, reading)) => {
                assert_eq!(reading.unwrap().token, 1);
                false
            }
            _ => false,
        }
    });
}

#[test]
fn the_engine_thread_is_named() {
    let (sender, names) = mpsc::channel();
    let engine = Engine::start(
        Box::new(FakeFactory::new(STEREO_44K)),
        no_microphone(),
        Box::new(move |_| {
            let name = thread::current().name().map(str::to_owned);
            let _ = sender.send(name);
        }),
    )
    .unwrap();

    engine.send(load(track(1, "no-such-file.flac")));

    let name = names.recv_timeout(DEADLINE).unwrap();
    assert_eq!(name.as_deref(), Some("aria-audio-engine"));
    assert!(engine.shutdown(DEADLINE));
}

// Exclusive mode.

fn device_format(sample_rate: u32, channels: u16, kind: SampleKind) -> DeviceFormat {
    DeviceFormat {
        sample_rate,
        channels,
        kind,
    }
}

fn state(path: OutputPath, device_rate: u32, device_bits: u16) -> OutputState {
    OutputState {
        path,
        device_rate,
        device_bits,
    }
}

/// `tone.flac` and the other 16-bit 44.1 kHz fixtures, as they should reach an exclusive output.
const CD_BIT_PERFECT: OutputState = OutputState {
    path: OutputPath::BitPerfect,
    device_rate: 44_100,
    device_bits: 16,
};

fn whole_second(token: u64) -> EngineEvent {
    TrackStarted {
        token,
        duration_ms: Some(1_000),
    }
}

fn quarter_second(token: u64) -> EngineEvent {
    TrackStarted {
        token,
        duration_ms: Some(250),
    }
}

/// The states among `events`, in order.
fn states(events: &[EngineEvent]) -> Vec<OutputState> {
    events
        .iter()
        .filter_map(|event| match event {
            Output(state) => Some(*state),
            _ => None,
        })
        .collect()
}

/// `events` without the states.
fn without_states(events: &[EngineEvent]) -> Vec<EngineEvent> {
    events
        .iter()
        .filter(|event| !matches!(event, Output(_)))
        .cloned()
        .collect()
}

/// Every sample of a file, as the decoder hands them over.
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

/// The frames of `channels` channels in which something sounds, one after the other. Where an
/// output ran dry the frames are silent, so this lines up what was heard with what a file holds.
fn sounding(samples: &[f32], channels: usize) -> Vec<f32> {
    samples
        .chunks_exact(channels)
        .filter(|frame| frame.iter().any(|sample| *sample != 0.0))
        .flatten()
        .copied()
        .collect()
}

/// `heard` holds every sample of the stereo `file`, exactly, in order, and nothing else.
fn assert_heard_whole(heard: &[f32], file: &Path) {
    assert_heard_exactly(heard, &decode_all(file), 2, file);
}

fn assert_heard_exactly(heard: &[f32], expected: &[f32], channels: usize, file: &Path) {
    let (heard, expected) = (sounding(heard, channels), sounding(expected, channels));
    assert!(
        heard == expected,
        "{} sounding samples heard, {} in {}",
        heard.len(),
        expected.len(),
        file.display()
    );
}

/// A mono file's samples as both channels carry them.
fn as_stereo(mono: &[f32]) -> Vec<f32> {
    mono.iter().flat_map(|sample| [*sample, *sample]).collect()
}

fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0, f32::max)
}

/// A quarter of a second of a 16-bit stereo tone at `rate`, in `dir`.
fn quarter_tone(dir: &Path, token: u64, rate: u32) -> TrackRequest {
    let path = dir.join(format!("{token}.wav"));
    if let Err(error) = write_tone_wav(&path, rate, 2, u64::from(rate / 4), 440.0) {
        panic!("no tone file: {error}");
    }
    TrackRequest {
        token,
        path,
        gain: 1.0,
    }
}

impl Rig {
    fn exclusive(format: OutputFormat) -> Self {
        let rig = Self::new(format);
        rig.send(SetExclusive(true));
        rig
    }

    /// Wait until the clock is on `token`, and return its position.
    fn position_on(&self, token: u64) -> u64 {
        wait_until("the clock to change tracks", || self.read().token == token);
        self.read().position_ms
    }
}

#[test]
fn exclusive_mode_opens_the_output_at_the_track_rate() {
    let rig = Rig::exclusive(STEREO_48K);

    rig.send(load(track(1, "tone.flac")));
    let output = rig.output(0);

    assert_eq!(
        output.opened_as(),
        (true, device_format(44_100, 2, SampleKind::I16))
    );
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);
    let (audio, events) = rig.pull_to_stop(&output);
    assert_eq!(events, [whole_second(1), TrackEnded { token: 1 }, Stopped]);
    assert_near(audible_frames(&audio), SECOND);
    assert!(rig.engine.shutdown(DEADLINE));
}

#[test]
fn a_rate_change_reopens_the_output_between_tracks() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::exclusive(STEREO_48K);
    rig.start_pair(
        quarter_tone(dir.path(), 1, 44_100),
        quarter_tone(dir.path(), 2, 96_000),
    );
    let mut events = Vec::new();

    let first = rig.output(0);
    let (first_audio, heard) = rig.pull_until(&first, |event| *event == TrackEnded { token: 1 });
    events.extend(heard);

    // The second track waited for the first to be heard out, and starts on an output of its own.
    let second = rig.output(1);
    assert!(first.is_closed());
    assert_eq!(
        second.opened_as(),
        (true, device_format(96_000, 2, SampleKind::I16))
    );
    assert_eq!(rig.position_on(2), 0);
    rig.send(SetNext(Some(quarter_tone(dir.path(), 3, 44_100))));
    let (second_audio, heard) = rig.pull_until(&second, |event| *event == TrackEnded { token: 2 });
    events.extend(heard);

    let third = rig.output(2);
    assert!(second.is_closed());
    assert_eq!(
        third.opened_as(),
        (true, device_format(44_100, 2, SampleKind::I16))
    );
    assert_eq!(rig.position_on(3), 0);
    let (third_audio, heard) = rig.pull_to_stop(&third);
    events.extend(heard);

    assert_eq!(
        without_states(&events),
        [
            quarter_second(1),
            TrackEnded { token: 1 },
            quarter_second(2),
            TrackEnded { token: 2 },
            quarter_second(3),
            TrackEnded { token: 3 },
            Stopped,
        ]
    );
    assert_eq!(
        states(&events),
        [
            CD_BIT_PERFECT,
            state(OutputPath::BitPerfect, 96_000, 16),
            CD_BIT_PERFECT,
        ]
    );
    // Each track whole, and on its own output only.
    assert_heard_whole(&first_audio, &dir.path().join("1.wav"));
    assert_heard_whole(&second_audio, &dir.path().join("2.wav"));
    assert_heard_whole(&third_audio, &dir.path().join("3.wav"));
    assert_eq!(rig.factory.outputs().len(), 3);
    let reading = rig.read();
    assert_eq!(reading.token, 3);
    assert!(reading.position_ms >= 240, "the clock said {reading:?}");
}

#[test]
fn a_rate_change_with_gapless_off_reopens_the_output_too() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::exclusive(STEREO_48K);
    rig.send(SetGapless(false));
    rig.send(load(quarter_tone(dir.path(), 1, 44_100)));
    rig.send(SetNext(Some(quarter_tone(dir.path(), 2, 96_000))));

    let first = rig.output(0);
    let (first_audio, mut events) =
        rig.pull_until(&first, |event| *event == TrackEnded { token: 1 });
    let second = rig.output(1);
    assert_eq!(second.opened_as().1.sample_rate, 96_000);
    assert_eq!(rig.position_on(2), 0);
    let (second_audio, heard) = rig.pull_to_stop(&second);
    events.extend(heard);

    assert_eq!(
        without_states(&events),
        [
            quarter_second(1),
            TrackEnded { token: 1 },
            quarter_second(2),
            TrackEnded { token: 2 },
            Stopped,
        ]
    );
    assert_heard_whole(&first_audio, &dir.path().join("1.wav"));
    assert_heard_whole(&second_audio, &dir.path().join("2.wav"));
}

#[test]
fn loading_a_track_of_another_rate_reopens_the_output() {
    let rig = Rig::exclusive(STEREO_48K);
    rig.start(track(1, "tone.flac"));
    let first = rig.output(0);
    rig.hear_start(&first);

    rig.send(load(track(2, "tone-48k.flac")));

    let second = rig.output(1);
    assert!(first.is_closed());
    assert_eq!(
        second.opened_as(),
        (true, device_format(48_000, 2, SampleKind::I16))
    );
    let (audio, events) = rig.pull_to_stop(&second);
    assert_eq!(
        without_states(&events),
        [whole_second(2), TrackEnded { token: 2 }, Stopped]
    );
    assert_eq!(states(&events), [state(OutputPath::BitPerfect, 48_000, 16)]);
    assert_heard_whole(&audio, &fixture("tone-48k.flac"));
}

#[test]
fn tracks_of_one_rate_stay_on_one_output() {
    let rig = Rig::exclusive(STEREO_48K);

    rig.start_pair(
        track(1, "silence-gap-a.flac"),
        track(2, "silence-gap-b.flac"),
    );
    let output = rig.output(0);
    let audio = output.pull(SECOND + 1_000);

    assert_eq!(output.opened_as().1.sample_rate, 44_100);
    let heard = trimmed(&audio);
    assert_near(heard.len() / 2, SECOND);
    assert!(longest_zero_run(heard) <= 2, "a gap at the join");
    assert!(largest_step(heard) < SMOOTH_STEP, "the waveform jumped");
    let (_, events) = rig.pull_to_stop(&output);
    assert_eq!(states(&events), [CD_BIT_PERFECT]);
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn an_unsupported_rate_opens_the_nearest_and_resamples() {
    let rig = Rig::new(STEREO_44K);
    rig.factory.set_exclusive_rates(vec![48_000, 96_000]);
    rig.send(SetExclusive(true));

    rig.send(load(track(1, "tone.flac")));
    let output = rig.output(0);

    assert_eq!(
        output.opened_as(),
        (true, device_format(48_000, 2, SampleKind::I16))
    );
    assert_eq!(rig.next_state(), state(OutputPath::Resampled, 48_000, 16));
    let (audio, events) = rig.pull_to_stop(&output);
    assert_eq!(events, [whole_second(1), TrackEnded { token: 1 }, Stopped]);
    assert_near(audible_frames(&audio), 48_000);
}

#[test]
fn a_refused_exclusive_open_falls_back_to_shared() {
    let rig = Rig::new(STEREO_48K);
    rig.factory.refuse_exclusive(true);
    rig.send(SetExclusive(true));

    rig.send(load(track(1, "tone.flac")));
    let output = rig.output(0);

    assert_eq!(
        output.opened_as(),
        (false, device_format(48_000, 2, SampleKind::I32))
    );
    // Both come before anything is heard.
    assert_eq!(rig.next_event(), ExclusiveUnavailable);
    assert_eq!(
        rig.next_event(),
        Output(state(OutputPath::SystemMixer, 48_000, 32))
    );
    let (audio, events) = rig.pull_to_stop(&output);
    // Once only, and the track plays.
    assert_eq!(events, [whole_second(1), TrackEnded { token: 1 }, Stopped]);
    assert_near(audible_frames(&audio), 48_000);
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn a_volume_of_zero_is_silent_only_on_a_shared_output() {
    // Exclusive, a volume of 0 plays at full level, as the locked slider shows; only mute silences.
    let rig = Rig::exclusive(STEREO_44K);
    rig.send(SetVolume(0.0));
    rig.start(track(1, "tone.flac"));
    let exclusive = rig.output(0);
    assert!(exclusive.opened_as().0);
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);
    assert!(
        audible_frames(&exclusive.pull(SECOND / 4)) > 0,
        "exclusive at 0 is silent"
    );
    rig.send(SetMuted(true));
    assert_eq!(rig.next_state().path, OutputPath::Converted);
    exclusive.pull(CHUNK_FRAMES);
    let muted = exclusive.pull(CHUNK_FRAMES);
    assert!(
        muted.iter().all(|sample| *sample == 0.0),
        "mute let sound through"
    );

    // Asked for exclusive and refused, the same 0 is silent from the first sample.
    let rig = Rig::new(STEREO_48K);
    rig.factory.refuse_exclusive(true);
    rig.send(SetExclusive(true));
    rig.send(SetVolume(0.0));
    rig.start(track(1, "tone.flac"));
    let shared = rig.output(0);
    assert!(!shared.opened_as().0);
    let audio = shared.pull(SECOND / 4);
    assert!(
        audio.iter().all(|sample| *sample == 0.0),
        "a shared output played a volume of 0 aloud"
    );
}

#[test]
fn each_condition_that_breaks_bit_perfect_is_reported() {
    // 24 bits on a device that takes only 16.
    let rig = Rig::exclusive(STEREO_48K);
    rig.factory.set_exclusive_kinds(vec![SampleKind::I16]);
    rig.send(load(track(1, "depth-24.flac")));
    assert_eq!(
        rig.output(0).opened_as(),
        (true, device_format(44_100, 2, SampleKind::I16))
    );
    assert_eq!(rig.next_state(), state(OutputPath::Converted, 44_100, 16));

    // The same file where 24 bits fit.
    let rig = Rig::exclusive(STEREO_48K);
    rig.send(load(track(1, "depth-24.flac")));
    assert_eq!(rig.next_state(), state(OutputPath::BitPerfect, 44_100, 24));

    // Mono on a device that takes only stereo: mono to both channels counts as unchanged.
    let rig = Rig::exclusive(STEREO_48K);
    rig.send(load(track(1, "tone-mono.wav")));
    assert_eq!(
        rig.output(0).opened_as(),
        (true, device_format(44_100, 2, SampleKind::I16))
    );
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);

    // A lossy file decodes to float.
    let rig = Rig::exclusive(STEREO_48K);
    rig.send(load(track(1, "tone.mp3")));
    assert_eq!(rig.next_state(), state(OutputPath::Converted, 44_100, 32));

    // A gain other than 1.0: only muting makes one while exclusive.
    let rig = Rig::exclusive(STEREO_48K);
    rig.send(load(track(1, "tone.flac")));
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);
    rig.send(SetMuted(true));
    assert_eq!(rig.next_state(), state(OutputPath::Converted, 44_100, 16));
    rig.send(SetMuted(false));
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);
}

#[test]
fn a_fade_out_lowers_the_music_to_silence_and_play_brings_it_back() {
    let rig = Rig::new(STEREO_44K);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long.wav");
    write_tone_wav(&path, 44_100, 2, 5 * 44_100, 440.0).unwrap();
    rig.start(TrackRequest {
        token: 1,
        path,
        gain: 1.0,
    });
    let output = rig.output(0);
    let loudest = |samples: &[f32]| samples.iter().fold(0.0f32, |most, s| most.max(s.abs()));
    let before = loudest(&output.pull(SECOND / 10));
    assert!(before > 0.1, "nothing was heard: {before}");

    rig.send(FadeOut(Duration::from_millis(300)));
    // Pulled in real time, the level falls, and is silence once the fade is over.
    let mut levels = Vec::new();
    for _ in 0..8 {
        thread::sleep(Duration::from_millis(60));
        levels.push(loudest(&output.pull(SECOND * 6 / 100)));
    }
    assert!(
        levels[1] < before && levels[2] < levels[0],
        "the level did not fall: {levels:?}"
    );
    assert_eq!(*levels.last().unwrap(), 0.0, "{levels:?}");

    // A pause and a play start over at the full level.
    rig.send(Pause);
    rig.resume();
    let mut after = 0.0f32;
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(20));
        after = after.max(loudest(&output.pull(SECOND / 50)));
    }
    assert!(after > before * 0.9, "the level stayed down: {after}");
}

#[test]
fn a_track_plays_at_its_own_gain() {
    let source = decode_all(&fixture("tone.flac"));
    let rig = Rig::new(STEREO_44K);
    rig.start(TrackRequest {
        gain: 0.5,
        ..track(1, "tone.flac")
    });
    let audio = rig.output(0).pull(SECOND / 4);
    let fade = 2 * SECOND * FADE_MS as usize / 1_000;
    let halved: Vec<f32> = source[fade..audio.len()].iter().map(|s| s * 0.5).collect();
    assert!(
        audio[fade..] == halved[..],
        "the track did not play at half"
    );
}

#[test]
fn a_track_gain_other_than_one_is_not_bit_perfect() {
    let rig = Rig::exclusive(STEREO_44K);
    rig.start(TrackRequest {
        gain: 0.5,
        ..track(1, "tone.flac")
    });
    assert!(rig.output(0).opened_as().0);
    assert_eq!(rig.next_state(), state(OutputPath::Converted, 44_100, 16));
}

#[test]
fn volume_is_unity_in_exclusive_mode() {
    let source = decode_all(&fixture("tone.flac"));
    let rig = Rig::exclusive(STEREO_44K);
    rig.send(SetVolume(0.5));
    rig.start(track(1, "tone.flac"));
    let exclusive = rig.output(0);
    assert!(exclusive.opened_as().0);
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);

    let audio = exclusive.pull(SECOND / 4);
    // Past whatever fade there was at the start.
    let fade = 2 * SECOND * FADE_MS as usize / 1_000;
    assert!(
        audio[fade..] == source[fade..audio.len()],
        "the samples are not the file's"
    );

    // Mute still silences.
    rig.send(SetMuted(true));
    assert_eq!(rig.next_state().path, OutputPath::Converted);
    // The gain ramps down within one buffer.
    exclusive.pull(CHUNK_FRAMES);
    let muted = exclusive.pull(CHUNK_FRAMES);
    assert!(
        muted.iter().all(|sample| *sample == 0.0),
        "mute let sound through"
    );
    rig.send(SetMuted(false));
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);

    // Shared again, the volume is back.
    rig.send(SetExclusive(false));
    let shared = rig.output(1);
    assert!(!shared.opened_as().0);
    assert_eq!(rig.next_state(), state(OutputPath::SystemMixer, 44_100, 32));
    rig.hold(1);
    rig.resume();
    let halved = shared.pull(CHUNK_FRAMES * 2);
    let level = peak(&halved[fade..]);
    let expected = peak(&source) / 2.0;
    assert!(
        (level - expected).abs() < 1e-6,
        "peak {level}, expected {expected}"
    );
}

#[test]
fn switching_the_mode_while_playing_carries_on_from_the_same_place() {
    let rig = Rig::new(STEREO_48K);
    rig.start(track(1, "tone.flac"));
    let shared = rig.output(0);
    assert!(!shared.opened_as().0);
    rig.pull_heard(&shared, 12_000, 240);
    assert!(matches!(rig.next_event(), TrackStarted { .. }));

    rig.send(SetExclusive(true));
    let exclusive = rig.output(1);
    assert!(shared.is_closed());
    assert_eq!(
        exclusive.opened_as(),
        (true, device_format(44_100, 2, SampleKind::I16))
    );
    rig.hold(1);
    let at = rig.read().position_ms;
    assert!(at.abs_diff(250) <= 100, "the clock said {at}");
    rig.resume();
    rig.pull_heard(&exclusive, 11_025, at + 240);

    rig.send(SetExclusive(false));
    let shared_again = rig.output(2);
    assert!(exclusive.is_closed());
    assert!(!shared_again.opened_as().0);
    rig.hold(1);
    let at = rig.read().position_ms;
    assert!(at.abs_diff(500) <= 100, "the clock said {at}");
    rig.resume();
    let (audio, events) = rig.pull_to_stop(&shared_again);

    assert_eq!(without_states(&events), [TrackEnded { token: 1 }, Stopped]);
    assert_eq!(
        states(&events),
        [CD_BIT_PERFECT, state(OutputPath::SystemMixer, 48_000, 32)]
    );
    assert_near(audible_frames(&audio), 24_000);
}

#[test]
fn a_seek_in_exclusive_mode_keeps_the_output() {
    let rig = Rig::exclusive(STEREO_48K);
    rig.start(track(1, "tone.flac"));
    let output = rig.output(0);
    rig.hear_start(&output);

    rig.send(Seek(500));
    wait_until("the seek", || rig.read().position_ms == 500);
    let (audio, events) = rig.pull_to_stop(&output);

    assert_eq!(events, [TrackEnded { token: 1 }, Stopped]);
    assert_near(audible_frames(&audio), SECOND / 2);
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn a_failed_exclusive_output_is_replaced_and_playback_carries_on() {
    let rig = Rig::exclusive(STEREO_48K);
    rig.start(track(1, "tone.flac"));
    let first = rig.output(0);
    rig.pull_heard(&first, SECOND / 2, 490);
    assert!(matches!(rig.next_track_event(), TrackStarted { .. }));
    let before = rig.read();

    first.fail();

    assert_eq!(rig.next_track_event(), DeviceFellBack);
    let second = rig.output(1);
    assert_eq!(
        second.opened_as(),
        (true, device_format(44_100, 2, SampleKind::I16))
    );
    assert!(rig.read().playing);
    rig.hold(1);
    let after = rig.read();
    assert!(
        after.position_ms.abs_diff(before.position_ms) <= 100,
        "{before:?} became {after:?}"
    );
    rig.resume();
    let (audio, events) = rig.pull_to_stop(&second);

    // The same state as before, so nothing new to say about it.
    assert_eq!(events, [TrackEnded { token: 1 }, Stopped]);
    assert_near(audible_frames(&audio), SECOND / 2);
}

#[test]
fn shared_mode_behaves_as_before() {
    let rig = Rig::new(STEREO_48K);

    rig.send(load(track(1, "tone.flac")));
    let output = rig.output(0);
    let (_, events) = rig.pull_to_stop(&output);

    assert_eq!(
        output.opened_as(),
        (false, device_format(48_000, 2, SampleKind::I32))
    );
    // Nothing about the output: with the mode off, the path is always the system mixer.
    assert_eq!(events, [whole_second(1), TrackEnded { token: 1 }, Stopped]);
}

fn tenth_second(token: u64) -> EngineEvent {
    TrackStarted {
        token,
        duration_ms: Some(100),
    }
}

#[test]
fn a_wider_depth_reopens_the_output_between_tracks() {
    let rig = Rig::exclusive(STEREO_48K);
    rig.factory
        .set_exclusive_kinds(vec![SampleKind::I16, SampleKind::I24In32]);
    rig.start_pair(track(1, "depth-16.flac"), track(2, "depth-24.flac"));

    let first = rig.output(0);
    assert_eq!(
        first.opened_as(),
        (true, device_format(44_100, 2, SampleKind::I16))
    );
    let (first_audio, mut events) =
        rig.pull_until(&first, |event| *event == TrackEnded { token: 1 });
    let second = rig.output(1);
    assert!(first.is_closed());
    assert_eq!(
        second.opened_as(),
        (true, device_format(44_100, 2, SampleKind::I24In32))
    );
    assert_eq!(rig.position_on(2), 0);
    let (second_audio, heard) = rig.pull_to_stop(&second);
    events.extend(heard);

    assert_eq!(
        without_states(&events),
        [
            tenth_second(1),
            TrackEnded { token: 1 },
            tenth_second(2),
            TrackEnded { token: 2 },
            Stopped,
        ]
    );
    assert_eq!(
        states(&events),
        [CD_BIT_PERFECT, state(OutputPath::BitPerfect, 44_100, 24)]
    );
    assert_heard_whole(&first_audio, &fixture("depth-16.flac"));
    assert_heard_whole(&second_audio, &fixture("depth-24.flac"));
    assert_eq!(rig.factory.outputs().len(), 2);
}

#[test]
fn a_narrower_depth_keeps_the_output() {
    let rig = Rig::exclusive(STEREO_48K);
    rig.factory
        .set_exclusive_kinds(vec![SampleKind::I16, SampleKind::I24In32]);
    rig.start_pair(track(1, "depth-24.flac"), track(2, "depth-16.flac"));

    let output = rig.output(0);
    let audio = output.pull(4_410 * 2 + 1_000);

    assert_eq!(
        output.opened_as(),
        (true, device_format(44_100, 2, SampleKind::I24In32))
    );
    let heard = trimmed(&audio);
    assert_near(heard.len() / 2, 4_410 * 2);
    assert!(longest_zero_run(heard) <= 2, "a gap at the join");
    let (_, events) = rig.pull_to_stop(&output);
    // 16 bits widen exactly into 24.
    assert_eq!(states(&events), [state(OutputPath::BitPerfect, 44_100, 24)]);
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn turning_exclusive_back_on_reports_the_state_again() {
    let rig = Rig::exclusive(STEREO_48K);
    rig.send(load(track(1, "tone.flac")));
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);
    rig.send(Stop);
    assert_eq!(rig.next_track_event(), Stopped);

    // Off and on again with nothing playing, so no state could be worked out in between.
    rig.send(SetExclusive(false));
    rig.send(SetExclusive(true));
    rig.send(load(track(2, "tone.flac")));

    assert_eq!(rig.next_state(), CD_BIT_PERFECT);
}

#[test]
fn a_lost_exclusive_device_reports_the_state_again_when_an_output_opens() {
    let rig = Rig::exclusive(STEREO_48K);
    rig.start(track(1, "tone.flac"));
    let first = rig.output(0);
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);

    // The rebuild fails too, so nothing is open.
    rig.factory.fail_open(1);
    first.fail();
    while rig.next_event() != NoDevice {}

    // Whoever listens forgot the path with the device: the same state is news again.
    rig.send(Play);
    rig.output(1);
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);
}

#[test]
fn a_stereo_track_after_a_mono_one_is_not_downmixed() {
    let rig = Rig::exclusive(STEREO_48K);
    rig.factory.set_exclusive_channels(vec![1, 2]);
    rig.start_pair(track(1, "tone-mono.wav"), track(2, "tone.flac"));
    let output = rig.output(0);

    // The mono track goes to both channels, so the stereo one can follow on the same output.
    assert_eq!(
        output.opened_as(),
        (true, device_format(44_100, 2, SampleKind::I16))
    );
    let (audio, events) = rig.pull_to_stop(&output);

    assert_eq!(
        without_states(&events),
        [
            whole_second(1),
            TrackEnded { token: 1 },
            whole_second(2),
            TrackEnded { token: 2 },
            Stopped,
        ]
    );
    assert_eq!(states(&events), [CD_BIT_PERFECT]);
    let mut expected = as_stereo(&decode_all(&fixture("tone-mono.wav")));
    expected.extend(decode_all(&fixture("tone.flac")));
    assert_heard_exactly(&audio, &expected, 2, &fixture("tone.flac"));
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn a_track_with_more_channels_reopens_a_mono_output() {
    let rig = Rig::exclusive(STEREO_48K);
    rig.factory.set_exclusive_channels(vec![1]);
    rig.start_pair(track(1, "tone-mono.wav"), track(2, "tone.flac"));
    let first = rig.output(0);
    assert_eq!(
        first.opened_as(),
        (true, device_format(44_100, 1, SampleKind::I16))
    );

    // From now on the device takes stereo too, as one whose stereo mode was busy would.
    rig.factory.set_exclusive_channels(vec![1, 2]);
    let (first_audio, mut events) =
        rig.pull_until(&first, |event| *event == TrackEnded { token: 1 });
    let second = rig.output(1);
    assert!(first.is_closed());
    assert_eq!(
        second.opened_as(),
        (true, device_format(44_100, 2, SampleKind::I16))
    );
    assert_eq!(rig.position_on(2), 0);
    let (second_audio, heard) = rig.pull_to_stop(&second);
    events.extend(heard);

    assert_eq!(
        without_states(&events),
        [
            whole_second(1),
            TrackEnded { token: 1 },
            whole_second(2),
            TrackEnded { token: 2 },
            Stopped,
        ]
    );
    // Both bit-perfect at 44.1/16, so the state never changed.
    assert_eq!(states(&events), [CD_BIT_PERFECT]);
    let mono = fixture("tone-mono.wav");
    assert_heard_exactly(&first_audio, &decode_all(&mono), 1, &mono);
    assert_heard_whole(&second_audio, &fixture("tone.flac"));
}

#[test]
fn a_seek_while_a_reopen_waits_plays_on_from_there() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::exclusive(STEREO_48K);
    // The first track is decoded to its end, so the second waits for an output of its own.
    rig.start_pair(
        quarter_tone(dir.path(), 1, 44_100),
        quarter_tone(dir.path(), 2, 96_000),
    );
    let output = rig.output(0);
    rig.hear_start(&output);

    rig.send(Seek(150));
    wait_until("the seek", || rig.read().position_ms.abs_diff(150) <= 5);
    let (first_audio, mut events) =
        rig.pull_until(&output, |event| *event == TrackEnded { token: 1 });
    let second = rig.output(1);
    assert_eq!(
        second.opened_as(),
        (true, device_format(96_000, 2, SampleKind::I16))
    );
    let (second_audio, heard) = rig.pull_to_stop(&second);
    events.extend(heard);

    assert_eq!(
        without_states(&events),
        [
            TrackEnded { token: 1 },
            quarter_second(2),
            TrackEnded { token: 2 },
            Stopped,
        ]
    );
    // The 5 ms fade out of what was queued, then the first track from 150 ms to its end.
    assert_near(audible_frames(&first_audio), 220 + 4_410);
    assert_heard_whole(&second_audio, &dir.path().join("2.wav"));
    assert_eq!(rig.factory.outputs().len(), 2);
}

#[test]
fn a_stop_while_a_reopen_waits_opens_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::exclusive(STEREO_48K);
    rig.start_pair(
        quarter_tone(dir.path(), 1, 44_100),
        quarter_tone(dir.path(), 2, 96_000),
    );
    let output = rig.output(0);
    rig.hear_start(&output);

    rig.send(Stop);

    assert_eq!(rig.next_track_event(), Stopped);
    wait_until("the clock to reset", || {
        rig.read() == ClockReading::default()
    });
    output.pull(SECOND);
    // The thread still answers, and nothing was waiting to start.
    rig.send(load(track(3, "no-such-file.flac")));
    assert_eq!(rig.next_track_event(), TrackFailed { token: 3 });
    assert_eq!(rig.factory.outputs().len(), 1);
    assert_eq!(rig.read().token, 0);
}

#[test]
fn a_reopen_for_a_new_rate_keeps_counting_failures() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::exclusive(STEREO_48K);
    rig.start_pair(
        quarter_tone(dir.path(), 1, 44_100),
        quarter_tone(dir.path(), 2, 96_000),
    );
    for rebuild in 0..3 {
        rig.output(rebuild).fail();
        assert_eq!(rig.next_track_event(), DeviceFellBack);
    }
    let last = rig.output(3);
    rig.pull_until(&last, |event| *event == TrackEnded { token: 1 });

    let reopened = rig.output(4);
    assert_eq!(reopened.opened_as().1.sample_rate, 96_000);
    reopened.fail();

    // Three rebuilds in the window already: a new format is no fresh start.
    assert_eq!(rig.next_track_event(), NoDevice);
    assert_eq!(rig.factory.outputs().len(), 5);
}

// Sounds.

fn second_device() -> DeviceId {
    DeviceId("second".into())
}

/// A 16-bit stereo sound of `frames` frames at `rate` in `dir`. At 0 Hz it is silent.
fn sound_file(dir: &Path, name: &str, rate: u32, frames: u64, freq: f32) -> PathBuf {
    let path = dir.join(format!("{name}.wav"));
    if let Err(error) = write_tone_wav(&path, rate, 2, frames, freq) {
        panic!("no sound file: {error}");
    }
    path
}

/// A quarter of a second at 660 Hz, at the rate of the fake outputs.
fn quarter_sound(dir: &Path) -> PathBuf {
    sound_file(dir, "sound", 44_100, 11_025, 660.0)
}

fn play_sound(token: u64, path: &Path, gain: f32, route: SoundRoute) -> EngineCommand {
    PlaySound {
        token,
        path: path.to_path_buf(),
        gain,
        route,
    }
}

fn is_sound_event(event: &EngineEvent) -> bool {
    matches!(
        event,
        SoundStarted { .. } | SoundEnded { .. } | SoundFailed { .. } | SoundDeviceFailed
    )
}

fn sound_events(events: &[EngineEvent]) -> Vec<EngineEvent> {
    events
        .iter()
        .filter(|e| is_sound_event(e))
        .cloned()
        .collect()
}

/// Samples in which the fade at the start of the music can still be heard.
const MUSIC_FADE: usize = 2 * SECOND * FADE_MS as usize / 1_000;

impl Rig {
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

    /// The next event about a sound or the second device, passing over the others.
    fn next_sound_event(&self) -> EngineEvent {
        loop {
            let event = self.next_event();
            if is_sound_event(&event) {
                return event;
            }
        }
    }
}

#[test]
fn a_sound_plays_on_the_music_output_over_the_music() {
    let dir = tempfile::tempdir().unwrap();
    let sound = quarter_sound(dir.path());
    let music = decode_all(&fixture("tone.flac"));
    let rig = Rig::new(STEREO_44K);
    rig.start(track(1, "tone.flac"));
    let output = rig.output(0);
    let lead = output.pull(CHUNK_FRAMES);

    rig.send(play_sound(7, &sound, 0.5, SoundRoute::Mine));
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 7 });
    let mixed = output.pull(SECOND / 4);

    let at = lead.len();
    let expected: Vec<f32> = music[at..at + mixed.len()]
        .iter()
        .zip(decode_all(&sound))
        .map(|(music, sound)| music + sound * 0.5)
        .collect();
    assert!(
        mixed == expected,
        "the output is not the music plus the sound"
    );
    // All of it has been heard, so it ends without another frame pulled.
    assert_eq!(rig.next_sound_event(), SoundEnded { token: 7 });
    assert!(rig.read().playing, "the music stopped");
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn a_sound_alone_opens_the_output_and_it_closes_when_idle() {
    let dir = tempfile::tempdir().unwrap();
    let sound = quarter_sound(dir.path());
    let rig = Rig::with_idle_grace(STEREO_44K, SHORT_GRACE);

    rig.send(play_sound(3, &sound, 1.0, SoundRoute::Mine));
    let output = rig.output(0);
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 3 });
    let (audio, events) = rig.pull_until(&output, |event| *event == SoundEnded { token: 3 });

    assert_heard_exactly(&audio, &decode_all(&sound), 2, &sound);
    assert_eq!(events, [SoundEnded { token: 3 }]);
    wait_until("the output to close", || output.is_closed());
    assert_eq!(rig.factory.outputs().len(), 1);
    assert_eq!(
        rig.read(),
        ClockReading::default(),
        "a sound moved the clock"
    );
}

#[test]
fn a_caf_sound_plays() {
    // The library scan passes CAF files over, but a sound may be one.
    let sound = fixture("tone.caf");
    let rig = Rig::new(STEREO_44K);

    rig.send(play_sound(1, &sound, 1.0, SoundRoute::Mine));
    let output = rig.output(0);
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });
    let (audio, _) = rig.pull_until(&output, |event| *event == SoundEnded { token: 1 });

    assert_near(audible_frames(&audio), SECOND);
}

#[test]
fn a_sound_for_the_second_device_leaves_the_music_stream_alone() {
    let dir = tempfile::tempdir().unwrap();
    let sound = quarter_sound(dir.path());
    let music = decode_all(&fixture("tone.flac"));
    let rig = Rig::exclusive(STEREO_44K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.start(track(1, "tone.flac"));
    let mine = rig.output_on(None);
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);

    rig.send(play_sound(5, &sound, 1.0, SoundRoute::Second));
    let second = rig.output_on(Some(&second_device()));
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 5 });
    let heard_music = mine.pull(SECOND / 2);
    let (heard_sound, events) = rig.pull_until(&second, |event| *event == SoundEnded { token: 5 });

    // Opened shared, whatever the music's mode.
    assert_eq!(
        second.opened_as(),
        (false, device_format(44_100, 2, SampleKind::I32))
    );
    assert!(
        heard_music[MUSIC_FADE..] == music[MUSIC_FADE..heard_music.len()],
        "the music changed"
    );
    assert_heard_exactly(&heard_sound, &decode_all(&sound), 2, &sound);
    assert!(states(&events).is_empty(), "the state changed: {events:?}");
    assert_eq!(rig.factory.outputs().len(), 2);
}

#[test]
fn both_plays_on_both_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let sound = quarter_sound(dir.path());
    let expected = decode_all(&sound);
    let rig = Rig::new(STEREO_44K);
    rig.send(SetSoundDevice(Some(second_device())));

    rig.send(play_sound(2, &sound, 1.0, SoundRoute::Both));
    let mine = rig.output_on(None);
    let second = rig.output_on(Some(&second_device()));
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 2 });
    let on_mine = mine.pull(SECOND / 2);
    let (on_second, _) = rig.pull_until(&second, |event| *event == SoundEnded { token: 2 });

    assert_heard_exactly(&on_mine, &expected, 2, &sound);
    assert_heard_exactly(&on_second, &expected, 2, &sound);
    assert_eq!(rig.factory.outputs().len(), 2);
}

#[test]
fn the_second_device_being_the_music_device_plays_once() {
    let dir = tempfile::tempdir().unwrap();
    let sound = quarter_sound(dir.path());
    let expected = decode_all(&sound);

    // The same device chosen for both.
    let rig = Rig::new(STEREO_44K);
    rig.send(SetDevice(Some(second_device())));
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(play_sound(1, &sound, 1.0, SoundRoute::Both));
    let output = rig.output(0);
    assert_eq!(output.device(), Some(second_device()));
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });
    let (audio, _) = rig.pull_until(&output, |event| *event == SoundEnded { token: 1 });
    // Once: two copies would sum to twice the level.
    assert_heard_exactly(&audio, &expected, 2, &sound);
    assert_eq!(rig.factory.outputs().len(), 1);

    // The music on the system default, which the second device is.
    let rig = Rig::new(STEREO_44K);
    rig.factory.set_devices(vec![DeviceInfo {
        id: second_device(),
        name: "Speakers".into(),
        is_default: true,
    }]);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(play_sound(1, &sound, 1.0, SoundRoute::Second));
    let output = rig.output(0);
    assert_eq!(output.device(), None);
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });
    let (audio, _) = rig.pull_until(&output, |event| *event == SoundEnded { token: 1 });
    assert_heard_exactly(&audio, &expected, 2, &sound);
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn no_second_device_plays_on_mine() {
    let dir = tempfile::tempdir().unwrap();
    let sound = quarter_sound(dir.path());
    for route in [SoundRoute::Second, SoundRoute::Both] {
        let rig = Rig::new(STEREO_44K);

        rig.send(play_sound(1, &sound, 1.0, route));
        let output = rig.output(0);
        assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });
        let (audio, _) = rig.pull_until(&output, |event| *event == SoundEnded { token: 1 });

        assert_eq!(output.device(), None, "{route:?}");
        assert_heard_exactly(&audio, &decode_all(&sound), 2, &sound);
        assert_eq!(rig.factory.outputs().len(), 1, "{route:?}");
    }
}

#[test]
fn a_second_device_that_cannot_open_reports_and_both_still_plays_on_mine() {
    let dir = tempfile::tempdir().unwrap();
    let sound = quarter_sound(dir.path());
    let rig = Rig::new(STEREO_44K);
    rig.factory.remove_device(second_device());
    rig.send(SetSoundDevice(Some(second_device())));

    rig.send(play_sound(1, &sound, 1.0, SoundRoute::Both));
    assert_eq!(rig.next_sound_event(), SoundDeviceFailed);
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });
    let output = rig.output(0);
    assert_eq!(output.device(), None);
    let (audio, _) = rig.pull_until(&output, |event| *event == SoundEnded { token: 1 });
    assert_heard_exactly(&audio, &decode_all(&sound), 2, &sound);

    // Only the second device was wanted, so nothing plays.
    rig.send(play_sound(2, &sound, 1.0, SoundRoute::Second));
    assert_eq!(rig.next_sound_event(), SoundDeviceFailed);
    assert_eq!(rig.next_sound_event(), SoundEnded { token: 2 });
    assert!(output.pull(SECOND / 4).iter().all(|sample| *sample == 0.0));
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn a_seventh_sound_stops_the_oldest() {
    let dir = tempfile::tempdir().unwrap();
    let loud = sound_file(dir.path(), "loud", 44_100, 2 * SECOND as u64, 440.0);
    let quiet = sound_file(dir.path(), "quiet", 44_100, 2 * SECOND as u64, 0.0);
    let rig = Rig::new(STEREO_44K);
    assert_eq!(SOUNDS_AT_ONCE, 6);

    rig.send(play_sound(1, &loud, 1.0, SoundRoute::Mine));
    for token in 2..=6 {
        rig.send(play_sound(token, &quiet, 1.0, SoundRoute::Mine));
    }
    let output = rig.output(0);
    for token in 1..=6 {
        assert_eq!(rig.next_sound_event(), SoundStarted { token });
    }
    assert!(
        peak(&output.pull(CHUNK_FRAMES)) > 0.4,
        "the oldest is not heard"
    );

    rig.send(play_sound(7, &quiet, 1.0, SoundRoute::Mine));
    assert_eq!(rig.next_sound_event(), SoundEnded { token: 1 });
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 7 });
    // Past its fade out, the oldest is gone, and the six others still play.
    output.pull(CHUNK_FRAMES);
    assert_eq!(
        peak(&output.pull(CHUNK_FRAMES)),
        0.0,
        "the oldest still plays"
    );
    rig.send(StopSounds);
    let ended: Vec<_> = (0..6).map(|_| rig.next_sound_event()).collect();
    let expected: Vec<_> = (2..=7).map(|token| SoundEnded { token }).collect();
    assert_eq!(ended, expected);
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn firing_a_playing_sound_restarts_it() {
    let dir = tempfile::tempdir().unwrap();
    let sound = sound_file(dir.path(), "sound", 44_100, 11_025, 440.0);
    let rig = Rig::new(STEREO_44K);
    rig.send(play_sound(1, &sound, 1.0, SoundRoute::Mine));
    let output = rig.output(0);
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });
    let first = output.pull(4_410);
    assert_near(audible_frames(&first), 4_410);

    rig.send(play_sound(1, &sound, 1.0, SoundRoute::Mine));
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });
    let (again, events) = rig.pull_until(&output, |event| *event == SoundEnded { token: 1 });

    // Ended once, at the end of the second start.
    assert_eq!(sound_events(&events), [SoundEnded { token: 1 }]);
    // The whole sound from its start, not the rest of it. It begins at once, while what was
    // queued fades out under it; waiting for that fade would add its 5 ms.
    let audible = audible_frames(&again);
    assert!(audible.abs_diff(11_025) <= 10, "{audible} frames heard");
    let (heard, file) = (sounding(&again, 2), sounding(&decode_all(&sound), 2));
    let tail = 2 * 10_000;
    assert!(
        heard[heard.len() - tail..] == file[file.len() - tail..],
        "the sound did not play to its end"
    );
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn stop_sounds_ends_them_all_and_not_the_music() {
    let dir = tempfile::tempdir().unwrap();
    let long = sound_file(dir.path(), "long", 44_100, 2 * SECOND as u64, 660.0);
    let music = decode_all(&fixture("tone.flac"));
    let rig = Rig::new(STEREO_44K);
    rig.start(track(1, "tone.flac"));
    let output = rig.output(0);
    let mut heard = output.pull(CHUNK_FRAMES);

    for token in 1..=3 {
        rig.send(play_sound(token, &long, 0.5, SoundRoute::Mine));
    }
    for token in 1..=3 {
        assert_eq!(rig.next_sound_event(), SoundStarted { token });
    }
    heard.extend(output.pull(CHUNK_FRAMES));
    rig.send(StopSounds);
    for token in 1..=3 {
        assert_eq!(rig.next_sound_event(), SoundEnded { token });
    }
    // The sounds fade out within this.
    heard.extend(output.pull(CHUNK_FRAMES));
    let after = output.pull(SECOND / 4);

    let at = heard.len();
    assert!(
        after == music[at..at + after.len()],
        "the music changed after the stop"
    );
    assert!(rig.read().playing, "the music stopped");
    assert_eq!(rig.read().token, 1);
}

#[test]
fn a_sound_that_cannot_be_opened_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.wav");
    let junk = dir.path().join("junk.wav");
    fs::write(&junk, b"this is not audio").unwrap();

    // With nothing playing, nothing opens.
    let rig = Rig::new(STEREO_44K);
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(play_sound(1, &missing, 1.0, SoundRoute::Mine));
    rig.send(play_sound(2, &junk, 1.0, SoundRoute::Both));
    assert_eq!(rig.next_event(), SoundFailed { token: 1 });
    assert_eq!(rig.next_event(), SoundFailed { token: 2 });
    assert!(rig.factory.outputs().is_empty());

    // Over music, the music plays on as it was.
    let music = decode_all(&fixture("tone.flac"));
    let rig = Rig::new(STEREO_44K);
    rig.start(track(1, "tone.flac"));
    let output = rig.output(0);
    let lead = output.pull(CHUNK_FRAMES);
    rig.send(play_sound(9, &missing, 1.0, SoundRoute::Mine));
    assert_eq!(rig.next_sound_event(), SoundFailed { token: 9 });
    let after = output.pull(SECOND / 4);

    let at = lead.len();
    assert!(after == music[at..at + after.len()], "the music changed");
    assert!(rig.read().playing);
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn a_sound_of_another_rate_is_resampled_to_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let sound = sound_file(dir.path(), "sound-48k", 48_000, 12_000, 440.0);
    let rig = Rig::new(STEREO_44K);

    rig.send(play_sound(1, &sound, 1.0, SoundRoute::Mine));
    let output = rig.output(0);
    let (audio, _) = rig.pull_until(&output, |event| *event == SoundEnded { token: 1 });

    // A quarter of a second at the output's rate, at the file's level.
    assert_near(audible_frames(&audio), 11_025);
    let level = peak(&audio);
    assert!((level - 0.5).abs() < 0.02, "peak {level}");
}

#[test]
fn a_sound_in_an_exclusive_stream_changes_the_state_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let sound = sound_file(dir.path(), "short", 44_100, 4_410, 660.0);
    let rig = Rig::exclusive(STEREO_48K);
    rig.start(track(1, "tone.flac"));
    let output = rig.output(0);
    assert_eq!(rig.next_state(), CD_BIT_PERFECT);

    rig.send(play_sound(1, &sound, 1.0, SoundRoute::Mine));
    assert_eq!(
        rig.next_state(),
        state(OutputPath::MixedWithSound, 44_100, 16)
    );
    // Past the end of the sound, and well short of the end of the music.
    output.pull(2 * 4_410);
    let mut events = Vec::new();
    while !matches!(events.last(), Some(Output(_))) {
        events.push(rig.next_event());
    }

    assert_eq!(sound_events(&events), [SoundEnded { token: 1 }]);
    assert_eq!(states(&events), [CD_BIT_PERFECT]);
    assert!(rig.read().playing, "the music stopped");
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn a_reopen_for_a_new_rate_cuts_sounds_on_that_output() {
    let dir = tempfile::tempdir().unwrap();
    let long = sound_file(dir.path(), "long", 44_100, 2 * SECOND as u64, 660.0);
    let rig = Rig::exclusive(STEREO_48K);
    rig.start_pair(
        quarter_tone(dir.path(), 1, 44_100),
        quarter_tone(dir.path(), 2, 96_000),
    );
    let first = rig.output(0);
    rig.send(play_sound(7, &long, 1.0, SoundRoute::Mine));
    rig.send(play_sound(8, &long, 1.0, SoundRoute::Mine));
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 7 });
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 8 });

    let (_, events) = rig.pull_until(&first, |event| *event == TrackEnded { token: 1 });
    assert!(sound_events(&events).is_empty(), "{events:?}");
    let second = rig.output(1);
    assert!(first.is_closed());
    assert_eq!(rig.next_sound_event(), SoundEnded { token: 7 });
    assert_eq!(rig.next_sound_event(), SoundEnded { token: 8 });
    let (audio, _) = rig.pull_to_stop(&second);

    // The new output holds the next track and nothing else.
    assert_heard_whole(&audio, &dir.path().join("2.wav"));
    assert_eq!(rig.factory.outputs().len(), 2);
}

#[test]
fn the_second_output_closes_after_its_grace() {
    let dir = tempfile::tempdir().unwrap();
    let sound = quarter_sound(dir.path());
    let rig = Rig::with_idle_grace(STEREO_44K, SHORT_GRACE);
    rig.send(SetSoundDevice(Some(second_device())));

    rig.send(play_sound(1, &sound, 1.0, SoundRoute::Second));
    let second = rig.output_on(Some(&second_device()));
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });
    rig.pull_until(&second, |event| *event == SoundEnded { token: 1 });

    wait_until("the second output to close", || second.is_closed());
    assert_eq!(rig.factory.outputs().len(), 1, "the music's output opened");
}

/// After the events so far, no other sound event is waiting: the next one is the failure of a
/// sound that cannot be opened, sent now.
fn assert_no_more_sound_events(rig: &Rig, dir: &Path) {
    rig.send(play_sound(
        99,
        &dir.join("missing.wav"),
        1.0,
        SoundRoute::Mine,
    ));
    assert_eq!(rig.next_sound_event(), SoundFailed { token: 99 });
}

#[test]
fn a_sound_takes_a_spare_input_that_is_free_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let sound = quarter_sound(dir.path());
    let long = sound_file(dir.path(), "long", 44_100, 2 * SECOND as u64, 440.0);
    let rig = Rig::new(STEREO_44K);
    rig.send(play_sound(1, &long, 1.0, SoundRoute::Mine));
    rig.send(play_sound(2, &sound, 1.0, SoundRoute::Mine));
    let output = rig.output(0);
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 2 });
    // The short sound is heard out, so its input is free at once.
    output.pull(SECOND / 2);
    assert_eq!(rig.next_sound_event(), SoundEnded { token: 2 });
    // The long one's input still has to fade out what it holds.
    rig.send(StopSounds);
    assert_eq!(rig.next_sound_event(), SoundEnded { token: 1 });

    rig.send(play_sound(3, &sound, 1.0, SoundRoute::Mine));
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 3 });
    let heard = output.pull(4_410);

    // On the free input it is heard straight away, not after the fade.
    assert_near(audible_frames(&heard), 4_410);
    assert_eq!(rig.factory.outputs().len(), 1);
}

#[test]
fn a_sound_cut_by_a_device_change_ends() {
    let dir = tempfile::tempdir().unwrap();
    let long = sound_file(dir.path(), "long", 44_100, 2 * SECOND as u64, 660.0);
    let rig = Rig::new(STEREO_44K);
    rig.send(play_sound(1, &long, 1.0, SoundRoute::Mine));
    let first = rig.output(0);
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });

    rig.send(SetDevice(Some(DeviceId("headphones".into()))));

    assert_eq!(rig.next_sound_event(), SoundEnded { token: 1 });
    wait_until("the output to close", || first.is_closed());
    assert_no_more_sound_events(&rig, dir.path());
}

#[test]
fn moving_the_music_onto_the_second_device_plays_sounds_once() {
    let dir = tempfile::tempdir().unwrap();
    let sound = quarter_sound(dir.path());
    let long = sound_file(dir.path(), "long", 44_100, 2 * SECOND as u64, 660.0);
    let headphones = DeviceId("headphones".into());
    let rig = Rig::new(STEREO_44K);
    // The second device is the system default.
    rig.factory.set_devices(vec![DeviceInfo {
        id: second_device(),
        name: "Speakers".into(),
        is_default: true,
    }]);
    rig.send(SetDevice(Some(headphones.clone())));
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(play_sound(1, &long, 1.0, SoundRoute::Both));
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });
    let mine = rig.output_on(Some(&headphones));
    let second = rig.output_on(Some(&second_device()));

    // The music moves to the system default, the second device.
    rig.send(SetDevice(None));

    // Cut off both outputs, so it ends once.
    assert_eq!(rig.next_sound_event(), SoundEnded { token: 1 });
    wait_until("the second output to close", || second.is_closed());
    assert!(mine.is_closed());
    rig.send(play_sound(2, &sound, 1.0, SoundRoute::Both));
    let output = rig.output_on(None);
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 2 });
    let (audio, _) = rig.pull_until(&output, |event| *event == SoundEnded { token: 2 });
    assert_heard_exactly(&audio, &decode_all(&sound), 2, &sound);
    assert_eq!(
        rig.factory.outputs_on(Some(&second_device())).len(),
        1,
        "the second device opened again"
    );
    assert_no_more_sound_events(&rig, dir.path());
}

#[test]
fn a_repeat_that_plays_nowhere_leaves_the_first_with_one_end() {
    let dir = tempfile::tempdir().unwrap();
    let long = sound_file(dir.path(), "long", 44_100, 2 * SECOND as u64, 660.0);
    let rig = Rig::new(STEREO_44K);
    rig.factory.remove_device(second_device());
    rig.send(SetSoundDevice(Some(second_device())));
    rig.send(play_sound(1, &long, 1.0, SoundRoute::Mine));
    let output = rig.output(0);
    assert_eq!(rig.next_sound_event(), SoundStarted { token: 1 });

    rig.send(play_sound(1, &long, 1.0, SoundRoute::Second));
    assert_eq!(rig.next_sound_event(), SoundDeviceFailed);

    // The first start plays on, and ends once.
    assert!(peak(&output.pull(CHUNK_FRAMES)) > 0.4, "the sound stopped");
    rig.send(StopSounds);
    assert_eq!(rig.next_sound_event(), SoundEnded { token: 1 });
    assert_no_more_sound_events(&rig, dir.path());
}
