//! The player engine, driven through the fake output.
//!
//! Nothing plays until a test pulls frames, so no timing here depends on the clock on the wall.
//! Every wait is on a condition, with a deadline that only a failure ever reaches.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use aria_audio::fake::{FakeControl, FakeFactory};
use aria_audio::testing::{write_gapless_pair, write_tone_wav};
use aria_audio::{
    AudioError, AudioOutput, ClockReading, DeviceId, Engine, EngineCommand, EngineEvent,
    EngineHandle, OutputFactory, OutputFormat, PlaybackClock, TrackRequest,
};
use tempfile::TempDir;

use EngineCommand::{Load, Pause, Play, Seek, SetDevice, SetGapless, SetNext, SetVolume, Stop};
use EngineEvent::{DeviceFellBack, NoDevice, Stopped, TrackEnded, TrackFailed, TrackStarted};

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

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn track(token: u64, name: &str) -> TrackRequest {
    TrackRequest {
        token,
        path: fixture(name),
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
        let factory = FakeFactory::new(format);
        let (sender, events) = mpsc::channel();
        let on_event = move |event| {
            let _ = sender.send(event);
        };
        let engine = match Engine::start(Box::new(factory.clone()), Box::new(on_event)) {
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
        let event = self.next_event();
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
                events.push(event);
                if last(&event) {
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
        TrackRequest { token, path }
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
    }));
    rig.send(SetNext(Some(TrackRequest {
        token: 2,
        path: gone.clone(),
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

    rig.send(load(TrackRequest { token: 1, path }));

    assert_eq!(rig.next_event(), TrackFailed { token: 1 });
}

#[test]
fn a_track_that_breaks_plays_what_it_can_then_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = damaged_m4a(dir.path(), 50, 0xFF).unwrap();
    let rig = Rig::new(STEREO_44K);
    rig.send(load(TrackRequest { token: 1, path }));
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
    rig.send(SetNext(Some(TrackRequest { token: 2, path })));
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
    rig.start(TrackRequest { token: 1, path });
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
    release: Receiver<()>,
}

impl OutputFactory for StuckFactory {
    fn open(&self, _device: Option<&DeviceId>) -> Result<Box<dyn AudioOutput>, AudioError> {
        let _ = self.reached.send(());
        let _ = self.release.recv();
        Err(AudioError::NoDevice)
    }
}

#[test]
fn shutdown_gives_up_on_a_stuck_thread() {
    let (reached_tx, reached) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let factory = StuckFactory {
        reached: reached_tx,
        release: release_rx,
    };
    let engine = Engine::start(Box::new(factory), Box::new(|_| {})).unwrap();
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
    let engine = Engine::start(Box::new(factory.clone()), Box::new(on_event)).unwrap();
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
