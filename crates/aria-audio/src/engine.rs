//! The player engine: one thread that decodes tracks into an output and says what is heard.
//!
//! Audio runs ahead of the ear by up to one ring, so what the decoder has reached says little.
//! All writes to one input share a sample index, and a list of `Segment`s says which track each
//! stretch of it belongs to. `consumed()` then gives what is being heard, and that alone drives the
//! clock and the started and ended events.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::capture::{CaptureFactory, CaptureReader};
use crate::clock::{ClockReading, PlaybackClock};
use crate::decode::{StreamInfo, TrackDecoder};
use crate::device::DeviceId;
use crate::error::AudioError;
use crate::mixer::InputHandle;
use crate::output::{AudioOutput, Opened, OutputFactory, OutputFormat, OutputRequest};
use crate::resample::{Resampler, convert_channels};
use crate::sample_format::{DeviceFormat, OutputState, PathFacts, SourceFormat, output_path};
use crate::share::{Microphone, MusicCopy, copy_gain, mic_gain};
use crate::sounds::{SOUNDS_AT_ONCE, SecondOutput, SoundRoute, Spares, Target, Voice};

/// How often the thread looks at the output while a track is active.
const TICK: Duration = Duration::from_millis(10);
/// How long the output may stay open while nothing is playing. An open stream keeps the device
/// awake, and may keep the machine from sleeping, so it is closed after this and opened again by
/// the next Play.
pub const IDLE_GRACE: Duration = Duration::from_secs(15);
/// After going idle, the thread keeps its regular look at the output this long, for the last audio
/// a callback was still playing when the pause came to show on the clock.
const IDLE_SETTLE: Duration = Duration::from_millis(100);
/// Rebuilding after an output failure stops at this many within [`REBUILD_WINDOW`]. Some errors
/// repeat for as long as a device is in a bad way, and each one fails the new output too.
/// How much of the music the copy may catch up on once the second output has opened: the
/// listener's ring holds about a second.
const BACKLOG: Duration = Duration::from_secs(2);
const MAX_REBUILDS: usize = 3;
const REBUILD_WINDOW: Duration = Duration::from_secs(10);

/// One track to play. The token names it in events and on the clock.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackRequest {
    pub token: u64,
    pub path: PathBuf,
    /// The factor to play the track at, as ReplayGain asks: 1.0 leaves it as it is. Anything
    /// but 1.0 makes the path other than bit-perfect.
    pub gain: f32,
}

/// What the engine can be told to do.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineCommand {
    /// Replace everything with `track`, from `start_ms`. Without `play`, this only gets ready and
    /// needs no output.
    Load {
        track: TrackRequest,
        start_ms: u64,
        play: bool,
    },
    /// Say what follows the track being heard, whether or not its audio is already in the ring.
    SetNext(Option<TrackRequest>),
    Play,
    Pause,
    /// Move within the track being heard. Past the end, this is the end of the track.
    Seek(u64),
    /// The music's volume, 0.0 to 1.0, on a shared output. An exclusive output plays at full
    /// level whatever it is, as the locked slider shows, so a volume of 0 is silent only where the
    /// output turns out shared.
    SetVolume(f32),
    /// Silence the music on every output, exclusive ones too, or let it play again.
    SetMuted(bool),
    /// Lower the music to silence over this long, as a sleep timer runs out. It stays silent
    /// until the next Pause or Play, which bring it back to its level. Sounds are not faded.
    FadeOut(Duration),
    SetDevice(Option<DeviceId>),
    SetGapless(bool),
    /// Ask for outputs that hold the device alone, at each track's own format. While an output is
    /// exclusive, the music plays at full level whatever the volume, unless it is muted. Tracks of one rate join as before; at a change of rate, or a depth wider than
    /// the output carries, the output is opened again once the track before has been heard out.
    SetExclusive(bool),
    /// Drop the track and what follows it. Sounds play on.
    Stop,
    /// Play the sound in `path` from its start, at `gain` (0.0 to 1.0), on the outputs `route`
    /// names. The token names it in events. A sound already playing under `token` starts again.
    /// With [`SOUNDS_AT_ONCE`] playing, the one that started first stops.
    PlaySound {
        token: u64,
        path: PathBuf,
        gain: f32,
        route: SoundRoute,
    },
    /// Stop every sound. The music plays on.
    StopSounds,
    /// The second device for sounds, or none.
    SetSoundDevice(Option<DeviceId>),
    /// Copy the music into the second device too, at `gain` (0.0 to 1.0), or stop copying it.
    /// The copy is taken before the volume, so the local volume and mute do not change it. It
    /// follows pause, seek and every track change, as it is made of the same audio. With no
    /// second device, or with the second device being the music's, there is no copy.
    SetShareMusic {
        on: bool,
        gain: f32,
    },
    /// Send the microphone `device` (`None`: the system default) into the second device at
    /// `gain` (0.0 to 2.0, clipped at full scale), or stop. The microphone is open exactly while
    /// this is on and there is a second device that is not the music's. It never reaches the
    /// music's output.
    SetMicrophone {
        on: bool,
        device: Option<DeviceId>,
        gain: f32,
    },
}

/// What the engine reports. Tracks start and end when they are heard, not when they are decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    TrackStarted {
        token: u64,
        duration_ms: Option<u64>,
    },
    TrackEnded {
        token: u64,
    },
    /// The track could not be opened or decoded. Nothing plays until the next command.
    TrackFailed {
        token: u64,
    },
    /// The output failed and was replaced, or the device just chosen with
    /// [`SetDevice`](EngineCommand::SetDevice) could not be opened. A chosen device that is missing
    /// when an output is opened for playback falls back to the default without this.
    DeviceFellBack,
    /// No output could be opened, or outputs keep failing. Playback is paused, and the next
    /// [`Play`](EngineCommand::Play) tries again.
    NoDevice,
    /// The last track ended, or a [`Stop`](EngineCommand::Stop) took effect.
    Stopped,
    /// How the track being heard reaches the device, when that changes. Sent while exclusive mode
    /// is on, and once more after it is turned off, for the first shared output with a track on
    /// it. With the mode off the path is always the system mixer, so nothing more is sent. After
    /// [`NoDevice`](Self::NoDevice) the next state is sent even if it is the same as the last.
    Output(OutputState),
    /// Exclusive mode was on, but the device would not be held, so a shared output was opened.
    /// Sent once for each output opened that way.
    ExclusiveUnavailable,
    /// A sound began to play, or began again from its start.
    SoundStarted {
        token: u64,
    },
    /// A sound finished, was stopped, was pushed out by a newer one, or was cut off because the
    /// output it played on closed (as at a reopen for a new rate). Also sent for a sound that
    /// played nowhere because no output it was meant for could be opened, unless a sound under
    /// the same token plays on, which ends later. Each start has exactly one end.
    SoundEnded {
        token: u64,
    },
    /// The sound could not be played: its file could not be opened or decoded, or it could not
    /// be set up on any output. Other sounds, a sound playing under the same token, and the music
    /// are left as they were. For a file that cannot be opened, no output is opened either.
    SoundFailed {
        token: u64,
    },
    /// The second device could not be opened for a sound, or failed while one played on it.
    SoundDeviceFailed,
    /// The microphone is open and goes into the second device. `name` is the device's, when it
    /// has one.
    MicrophoneOpened {
        name: Option<String>,
    },
    /// The microphone was closed: turned off, or its second device went away or changed.
    MicrophoneClosed,
    /// The microphone could not be opened, or was lost. `denied` when the operating system
    /// refused it. It is tried again at the next change of the microphone or the devices.
    MicrophoneFailed {
        denied: bool,
    },
    /// The microphone asked for (or the system default) is a virtual cable, so it was not
    /// opened: it would feed the chat's sound back into itself.
    MicrophoneIsCable,
}

/// Starts the engine thread.
pub struct Engine;

impl Engine {
    /// Start the thread. `capture` opens the microphone, and only when it is asked for.
    /// `on_event` runs on the thread, so it must not block.
    pub fn start(
        factory: Box<dyn OutputFactory>,
        capture: Box<dyn CaptureFactory>,
        on_event: Box<dyn Fn(EngineEvent) + Send>,
    ) -> io::Result<EngineHandle> {
        Self::start_with_idle_grace(factory, capture, on_event, IDLE_GRACE)
    }

    /// Like [`start`](Self::start), but closes an idle output after `idle_grace` instead of
    /// [`IDLE_GRACE`]. For tests, which would not wait that long.
    pub fn start_with_idle_grace(
        factory: Box<dyn OutputFactory>,
        capture: Box<dyn CaptureFactory>,
        on_event: Box<dyn Fn(EngineEvent) + Send>,
        idle_grace: Duration,
    ) -> io::Result<EngineHandle> {
        let (commands, received) = mpsc::channel();
        let (done_tx, done) = mpsc::sync_channel(1);
        let clock = Arc::new(PlaybackClock::default());
        let worker_clock = Arc::clone(&clock);
        let thread = thread::Builder::new()
            .name("aria-audio-engine".into())
            .spawn(move || {
                let _done = DoneGuard {
                    done: done_tx,
                    clock: Arc::clone(&worker_clock),
                };
                Worker::new(factory, capture, on_event, worker_clock, idle_grace).run(&received);
            })?;
        Ok(EngineHandle {
            commands,
            clock,
            done,
            thread,
        })
    }
}

/// The way in to a running engine. Nothing here blocks except [`shutdown`](Self::shutdown), for as
/// long as it is told to.
pub struct EngineHandle {
    commands: Sender<EngineCommand>,
    clock: Arc<PlaybackClock>,
    done: Receiver<()>,
    thread: JoinHandle<()>,
}

impl EngineHandle {
    /// Queue a command.
    pub fn send(&self, cmd: EngineCommand) {
        // Fails only once the thread has exited, when nothing is listening.
        let _ = self.commands.send(cmd);
    }

    /// The clock the engine publishes to.
    pub fn clock(&self) -> Arc<PlaybackClock> {
        Arc::clone(&self.clock)
    }

    /// Stop the thread, which closes the output, and wait up to `timeout` for it.
    ///
    /// Returns `true` if it exited cleanly in time. Returns `false` if it timed out, in which case
    /// the thread is detached and left to finish on its own, or if it panicked.
    pub fn shutdown(self, timeout: Duration) -> bool {
        let Self {
            commands,
            done,
            thread,
            ..
        } = self;
        // Hanging up is the signal: the thread's next wait for a command fails.
        drop(commands);
        match done.recv_timeout(timeout) {
            // A disconnect also means the done guard was dropped.
            Ok(()) | Err(RecvTimeoutError::Disconnected) => match thread.join() {
                Ok(()) => true,
                Err(_) => {
                    tracing::error!("the audio engine panicked");
                    false
                }
            },
            Err(RecvTimeoutError::Timeout) => {
                tracing::warn!(
                    ?timeout,
                    "the audio engine did not stop in time; detaching it"
                );
                false
            }
        }
    }
}

/// Owned by the engine thread, and dropped after the worker. Its drop signals
/// [`EngineHandle::shutdown`] and stops the clock, so a panicking thread does both too.
struct DoneGuard {
    done: SyncSender<()>,
    clock: Arc<PlaybackClock>,
}

impl Drop for DoneGuard {
    fn drop(&mut self) {
        let last = self.clock.read();
        self.clock.publish(ClockReading {
            playing: false,
            ..last
        });
        // This is the only send on a capacity-1 channel, so it never blocks. It fails only if the
        // handle is gone and nobody is waiting.
        let _ = self.done.try_send(());
    }
}

/// `value * to / from`, rounded down, without overflowing on the way.
fn scale(value: u64, to: u32, from: u32) -> u64 {
    let scaled = u128::from(value) * u128::from(to) / u128::from(from.max(1));
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

/// Let go of commands that a later one in the same batch overrides.
///
/// Only the last `Seek` counts, as only the last `Load` and `SetNext` do, and a `Load` or `Stop`
/// overrides what came before it. Besides saving work, this keeps the clears of several seeks out
/// of one fade, which the mixer could only splice together at reduced level. Other pairs, such as
/// `[Load, Seek]`, still clear twice, which is a soft splice at worst.
///
/// All commands run before any decoding, so a dropped one loses only its immediate effects. The one
/// that shows: `[Load, Stop]` from idle sends no `Stopped`, as no track was ever there to stop.
fn coalesce(batch: Vec<EngineCommand>) -> Vec<EngineCommand> {
    let (mut seek_done, mut next_done, mut load_done) = (false, false, false);
    let mut kept: Vec<_> = batch
        .into_iter()
        .rev()
        .filter(|command| match command {
            EngineCommand::Seek(_) => !std::mem::replace(&mut seek_done, true),
            EngineCommand::SetNext(_) => !std::mem::replace(&mut next_done, true),
            EngineCommand::Load { .. } => {
                (seek_done, next_done) = (true, true);
                !std::mem::replace(&mut load_done, true)
            }
            EngineCommand::Stop => {
                (seek_done, next_done, load_done) = (true, true, true);
                true
            }
            _ => true,
        })
        .collect();
    kept.reverse();
    kept
}

/// An open output and the input the engine writes the music to.
struct Sink {
    input: InputHandle,
    /// Inputs for sounds that none plays through now.
    spares: Spares,
    output: Box<dyn AudioOutput>,
    format: OutputFormat,
    /// Whether the device is held exclusively.
    exclusive: bool,
    /// The format the device runs at.
    device: DeviceFormat,
    /// The track the output was opened for, if it was opened for one.
    opened_for: Option<SourceFormat>,
    /// Whether the chosen device could not be opened, so this is the system default.
    fell_back: bool,
}

impl Sink {
    /// Whether a track of `source` can follow on this output as it is. A shared output takes
    /// anything and resamples. An exclusive one keeps to the rate it was opened for, or that it
    /// runs at, to integer depths its samples carry exactly, and to as many channels as it has.
    fn fits(&self, source: &SourceFormat) -> bool {
        if !self.exclusive {
            return true;
        }
        let rate_fits = self.opened_for.map(|opened| opened.sample_rate)
            == Some(source.sample_rate)
            || self.device.sample_rate == source.sample_rate;
        rate_fits && !self.too_narrow_for(source) && !self.too_few_channels_for(source)
    }

    /// Whether `source` has more channels than the output, which would mix it down, and a new
    /// output might have them. As with depth, an output opened for a track with that many
    /// already got what the device gives.
    fn too_few_channels_for(&self, source: &SourceFormat) -> bool {
        let opened_channels = self.opened_for.map_or(0, |opened| opened.channels);
        source.channels > self.device.channels && source.channels > opened_channels
    }

    /// Whether `source` is integer audio deeper than the output carries, and a new output might
    /// carry it. If the output was opened for a track at least that deep, the device already
    /// gave what it could, and opening again would only cost the gapless join.
    fn too_narrow_for(&self, source: &SourceFormat) -> bool {
        let Some(bits) = source.bits.filter(|_| !source.float) else {
            return false;
        };
        let opened_bits = self
            .opened_for
            .filter(|opened| !opened.float)
            .and_then(|opened| opened.bits)
            .unwrap_or(0);
        bits > self.device.kind.bits() && bits > opened_bits
    }
}

/// What the decoder says about a track, as an output needs it.
fn source_format(info: &StreamInfo) -> SourceFormat {
    SourceFormat {
        sample_rate: info.sample_rate,
        channels: info.channels,
        bits: info.bits_per_sample,
        float: info.float,
    }
}

/// The stretch of the sample index that belongs to one track.
struct Segment {
    track: TrackRequest,
    /// Where the track's audio starts.
    start_index: u64,
    /// Where it stops, once that is known.
    end_index: Option<u64>,
    /// The frame of the file at `start_index`.
    source_start_frame: u64,
    /// The file's sample rate.
    rate: u32,
    /// The file's format. `None` for a track that could not be opened.
    source: Option<SourceFormat>,
    duration_ms: Option<u64>,
    /// Whether `TrackStarted` has been sent.
    started: bool,
    /// Reaching the end reports a failure instead of `TrackEnded`.
    failed: bool,
}

impl Segment {
    fn new(
        track: TrackRequest,
        info: &StreamInfo,
        start_index: u64,
        source_start_frame: u64,
        started: bool,
    ) -> Self {
        let rate = info.sample_rate;
        Self {
            track,
            start_index,
            end_index: None,
            source_start_frame,
            rate,
            source: Some(source_format(info)),
            duration_ms: info.total_frames.map(|total| scale(total, 1_000, rate)),
            started,
            failed: false,
        }
    }

    /// A track that could not be opened, to be reported once the audio before it has been heard.
    fn unplayable(track: TrackRequest, index: u64) -> Self {
        Self {
            track,
            start_index: index,
            end_index: Some(index),
            source_start_frame: 0,
            rate: 1,
            source: None,
            duration_ms: None,
            started: false,
            failed: true,
        }
    }

    fn is_unplayable(&self) -> bool {
        self.failed && self.end_index == Some(self.start_index)
    }

    /// The frame of the file being heard when `consumed` samples have been.
    fn source_frame(&self, consumed: u64, format: Option<OutputFormat>) -> u64 {
        let Some(format) = format else {
            return self.source_start_frame;
        };
        let heard_to = self.end_index.map_or(consumed, |end| consumed.min(end));
        let frames = heard_to.saturating_sub(self.start_index) / u64::from(format.channels.max(1));
        let source_frames = scale(frames, self.rate, format.sample_rate);
        self.source_start_frame.saturating_add(source_frames)
    }

    fn position_ms(&self, consumed: u64, format: Option<OutputFormat>) -> u64 {
        let position = scale(self.source_frame(consumed, format), 1_000, self.rate);
        self.duration_ms.map_or(position, |d| position.min(d))
    }
}

/// The track being decoded.
struct Feed {
    decoder: TrackDecoder,
    /// The gain of the track being decoded, applied before the resampler, so that what a join
    /// leaves inside it keeps the gain of the track before.
    gain: f32,
    /// Built when there is an output, whose format it depends on.
    resampler: Option<Resampler>,
    /// Frames read from the decoder for the current segment.
    frames_read: u64,
}

impl Feed {
    fn new(decoder: TrackDecoder, gain: f32) -> Self {
        Self {
            decoder,
            gain: track_gain(gain),
            resampler: None,
            frames_read: 0,
        }
    }

    /// Decode one block into `out` in the output's format. False at the end of the file.
    fn read(
        &mut self,
        format: OutputFormat,
        mixed: &mut Vec<f32>,
        out: &mut Vec<f32>,
    ) -> Result<bool, AudioError> {
        let (rate, channels) = (
            self.decoder.info().sample_rate,
            self.decoder.info().channels,
        );
        let resampler = match &mut self.resampler {
            Some(resampler) => resampler,
            empty => empty.insert(Resampler::new(rate, format.sample_rate, format.channels)?),
        };
        let Some(block) = self.decoder.next_block()? else {
            return Ok(false);
        };
        self.frames_read += (block.len() / usize::from(channels)) as u64;
        mixed.clear();
        convert_channels(block, channels, format.channels, mixed);
        if self.gain != 1.0 {
            for sample in mixed.iter_mut() {
                *sample *= self.gain;
            }
        }
        resampler.process(mixed, out)?;
        Ok(true)
    }
}

/// A place to carry on from.
struct Heard {
    track: TrackRequest,
    frame: u64,
    started: bool,
}

/// Everything the engine thread owns. Tracks in `segments` run from the one being heard, at the
/// front, to the one being decoded, at the back.
/// Whether the second output can take something now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecondReady {
    Open,
    /// Being opened: what needs it waits.
    Opening,
    /// No second output can be had now.
    Unavailable,
}

/// A sound fired, and what it was fired with.
struct WaitingSound {
    token: u64,
    decoder: TrackDecoder,
    gain: f32,
    route: SoundRoute,
}

/// A track's gain as the engine plays it: 0.0 to 10.0 (20 dB, the most ReplayGain lifts), and 1.0
/// for one that is not a number.
fn track_gain(gain: f32) -> f32 {
    if gain.is_finite() {
        gain.clamp(0.0, 10.0)
    } else {
        1.0
    }
}

/// Drop an output on a thread of its own, since closing a device can take seconds and the music
/// must not wait for it. If no thread can be started, it is dropped here.
fn let_go(output: Box<dyn AudioOutput>) {
    let closing = thread::Builder::new()
        .name("aria-audio-close".into())
        .spawn(move || drop(output));
    if let Err(error) = closing {
        tracing::debug!(%error, "closed an output on the engine thread");
    }
}

struct Worker {
    factory: Arc<dyn OutputFactory>,
    capture: Arc<dyn CaptureFactory>,
    on_event: Box<dyn Fn(EngineEvent) + Send>,
    /// Events of this pass, in order.
    events: Vec<EngineEvent>,
    clock: Arc<PlaybackClock>,
    device: Option<DeviceId>,
    volume: f32,
    muted: bool,
    /// A fade out under way: when it began and how long it takes.
    fade: Option<(Instant, Duration)>,
    gapless: bool,
    /// Whether exclusive outputs are asked for.
    exclusive: bool,
    playing: bool,
    sink: Option<Sink>,
    feed: Option<Feed>,
    next: Option<TrackRequest>,
    /// The next track, already open, which the exclusive output cannot play at its rate. It
    /// starts on a new output once everything queued has been heard.
    waiting: Option<(TrackRequest, TrackDecoder)>,
    /// The last state sent in [`EngineEvent::Output`], while there is one to compare against.
    last_output: Option<OutputState>,
    segments: VecDeque<Segment>,
    /// Audio in the output's format still to go into the ring, from `pending_from` on.
    pending: Vec<f32>,
    pending_from: usize,
    /// Scratch space between the channel conversion and the resampler.
    mixed: Vec<f32>,
    /// When recent rebuilds after an output failure happened.
    rebuilds: VecDeque<Instant>,
    reading: ClockReading,
    /// How long an output may stay open without sound.
    idle_grace: Duration,
    /// Since when the output has been open without sound, if it has been.
    idle_since: Option<Instant>,
    /// The handle is gone, so the thread ends.
    shut_down: bool,
    /// The sounds playing, the one that started first at the front.
    voices: VecDeque<Voice>,
    /// The second device for sounds.
    sound_device: Option<DeviceId>,
    /// The output on the second device, while it is open.
    second: Option<SecondOutput>,
    /// The second output being opened on a thread of its own, whose result comes here. While it
    /// is set, the engine looks for the result every tick.
    second_opening: Option<Receiver<Result<Opened, AudioError>>>,
    /// Sounds for the second output that wait for it to open, oldest first.
    waiting_sounds: Vec<WaitingSound>,
    /// Since when the second output has been open without a sound, if it has been.
    second_idle_since: Option<Instant>,
    /// Whether the music is to be copied into the second output.
    share_music: bool,
    share_gain: f32,
    /// The copy of the music on the second output, while there is one.
    copy: Option<MusicCopy>,
    /// What the listener's output took while sharing is on and there is no copy, in the
    /// listener's format: the copy starts with what of it is not yet heard, so that the chat hears
    /// a track from its start, however late the copy can start. At least [`BACKLOG`] of it is
    /// kept, and at most twice that.
    copy_backlog: Option<Vec<f32>>,
    /// Something changed that may let the copy start. It is tried on the next pass, and not
    /// again until the next such change, so a device that cannot be opened is not retried on
    /// every pass.
    share_dirty: bool,
    /// Whether the microphone is to be sent into the second output, from which device, and how
    /// loud.
    mic_on: bool,
    mic_device: Option<DeviceId>,
    mic_gain: f32,
    /// The microphone, while it is open.
    mic: Option<Microphone>,
    /// A microphone being opened on a thread of its own, whose result comes here. While it is
    /// set, the engine looks for the result every tick.
    mic_opening: Option<Receiver<Result<CaptureReader, AudioError>>>,
    /// As `share_dirty`, for the microphone.
    mic_dirty: bool,
    /// The microphone was closed because the second output failed, and opens again with it.
    mic_lost_with_second: bool,
}

impl Worker {
    fn new(
        factory: Box<dyn OutputFactory>,
        capture: Box<dyn CaptureFactory>,
        on_event: Box<dyn Fn(EngineEvent) + Send>,
        clock: Arc<PlaybackClock>,
        idle_grace: Duration,
    ) -> Self {
        Self {
            factory: Arc::from(factory),
            capture: Arc::from(capture),
            on_event,
            events: Vec::new(),
            clock,
            device: None,
            volume: 1.0,
            muted: false,
            fade: None,
            gapless: true,
            exclusive: false,
            playing: false,
            sink: None,
            feed: None,
            next: None,
            waiting: None,
            last_output: None,
            segments: VecDeque::new(),
            pending: Vec::new(),
            pending_from: 0,
            mixed: Vec::new(),
            rebuilds: VecDeque::new(),
            reading: ClockReading::default(),
            idle_grace,
            idle_since: None,
            shut_down: false,
            voices: VecDeque::new(),
            sound_device: None,
            second: None,
            second_opening: None,
            waiting_sounds: Vec::new(),
            second_idle_since: None,
            share_music: false,
            share_gain: 0.0,
            copy: None,
            copy_backlog: None,
            share_dirty: false,
            mic_on: false,
            mic_device: None,
            mic_gain: 1.0,
            mic: None,
            mic_opening: None,
            mic_dirty: false,
            mic_lost_with_second: false,
        }
    }

    /// How long to wait for a command: the regular look while sound is wanted or has just stopped,
    /// then until an idle output is due to close, and for as long as it takes without an output.
    fn wait_for_command(&mut self, commands: &Receiver<EngineCommand>) -> Option<EngineCommand> {
        let music_busy = (self.playing && self.has_track()) || self.sound_on(Target::Music);
        let idle = self.sink.is_some() && !music_busy;
        if !idle {
            self.idle_since = None;
        }
        let music_timeout = match (&self.sink, idle) {
            (None, _) => None,
            (Some(_), false) => Some(TICK),
            (Some(_), true) => {
                let idle_for = self.idle_since.get_or_insert_with(Instant::now).elapsed();
                if idle_for < IDLE_SETTLE.min(self.idle_grace) {
                    Some(TICK)
                } else {
                    Some(self.idle_grace.saturating_sub(idle_for))
                }
            }
        };
        let second_idle = self.second.is_some() && !self.second_needed();
        if !second_idle {
            self.second_idle_since = None;
        }
        let second_timeout = match (&self.second, second_idle) {
            (None, _) => None,
            (Some(_), false) => Some(TICK),
            (Some(_), true) => {
                let idle_for = self
                    .second_idle_since
                    .get_or_insert_with(Instant::now)
                    .elapsed();
                Some(self.idle_grace.saturating_sub(idle_for))
            }
        };
        // A microphone or a second output being opened is looked for every tick.
        let opening = self.mic_opening.is_some() || self.second_opening.is_some();
        let opening_timeout = opening.then_some(TICK);
        let timeout = music_timeout
            .into_iter()
            .chain(second_timeout)
            .chain(opening_timeout)
            .min();
        match timeout {
            Some(timeout) => match commands.recv_timeout(timeout) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    self.shut_down = true;
                    None
                }
            },
            None => match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => {
                    self.shut_down = true;
                    None
                }
            },
        }
    }

    /// Closes an output that has had no sound for the grace period. A track left loaded is kept at
    /// the position it was heard at, and the next Play opens an output there.
    fn close_idle_output(&mut self) {
        let due = self
            .idle_since
            .is_some_and(|since| since.elapsed() >= self.idle_grace);
        if due && self.sink.is_some() {
            tracing::debug!("closing the idle output");
            self.replace_output(false);
            self.idle_since = None;
        }
        let second_due = self
            .second_idle_since
            .is_some_and(|since| since.elapsed() >= self.idle_grace);
        if second_due && self.second.is_some() {
            tracing::debug!("closing the idle second output");
            self.close_second_output();
        }
    }

    fn run(mut self, commands: &Receiver<EngineCommand>) {
        loop {
            let first = self.wait_for_command(commands);
            if self.shut_down {
                return;
            }
            let batch = first.into_iter().chain(commands.try_iter()).collect();
            for command in coalesce(batch) {
                self.handle(command);
            }
            if self.fade.is_some() {
                self.apply_gain();
            }
            self.check_output();
            self.close_idle_output();
            self.finish_second_output();
            self.sync_copy();
            self.finish_microphone();
            self.sync_microphone();
            self.fill();
            self.flush_copy();
            self.feed_microphone();
            self.feed_sounds();
            self.settle();
            self.settle_sounds();
            self.report_output();
            // In this order, so that whoever hears of an event finds the clock agreeing with it.
            self.publish();
            for event in self.events.drain(..) {
                (self.on_event)(event);
            }
        }
    }

    fn handle(&mut self, command: EngineCommand) {
        match command {
            EngineCommand::Load {
                track,
                start_ms,
                play,
            } => self.load(track, start_ms, play),
            EngineCommand::SetNext(next) => self.set_next(next),
            EngineCommand::Play => {
                self.share_dirty = true;
                if self.has_track() {
                    self.set_playing(true);
                    self.ensure_output();
                }
            }
            EngineCommand::Pause => self.set_playing(false),
            EngineCommand::Seek(position_ms) => self.seek(position_ms),
            EngineCommand::SetVolume(volume) => {
                self.volume = volume;
                self.apply_gain();
            }
            EngineCommand::SetMuted(muted) => {
                self.muted = muted;
                self.apply_gain();
            }
            EngineCommand::FadeOut(over) => {
                self.fade = Some((Instant::now(), over));
                self.apply_gain();
            }
            EngineCommand::SetDevice(device) => {
                self.share_dirty = true;
                self.mic_dirty = true;
                self.set_device(device);
            }
            EngineCommand::SetGapless(gapless) => self.gapless = gapless,
            EngineCommand::SetExclusive(exclusive) => self.set_exclusive(exclusive),
            EngineCommand::Stop => self.stop(),
            EngineCommand::PlaySound {
                token,
                path,
                gain,
                route,
            } => self.play_sound(token, &path, gain, route),
            EngineCommand::StopSounds => self.stop_sounds(),
            EngineCommand::SetSoundDevice(device) => {
                self.share_dirty = true;
                self.mic_dirty = true;
                self.set_sound_device(device);
            }
            EngineCommand::SetShareMusic { on, gain } => self.set_share_music(on, gain),
            EngineCommand::SetMicrophone { on, device, gain } => {
                self.set_microphone(on, device, gain);
            }
        }
    }

    /// Queue an event, to go out at the end of the pass.
    fn emit(&mut self, event: EngineEvent) {
        self.events.push(event);
    }

    fn has_track(&self) -> bool {
        !self.segments.is_empty()
    }

    fn format(&self) -> Option<OutputFormat> {
        self.sink.as_ref().map(|sink| sink.format)
    }

    /// The music gain on an output. Muted, nothing; an exclusive one plays at exactly 1.0; a
    /// shared one at the volume.
    fn gain(&self, exclusive: bool) -> f32 {
        let level = if self.muted {
            0.0
        } else if exclusive {
            1.0
        } else if self.volume.is_finite() {
            self.volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
        level * self.fade_factor()
    }

    /// How far a fade out has left the music: 1.0 without one, down to 0.0 at its end.
    fn fade_factor(&self) -> f32 {
        let Some((started, over)) = self.fade else {
            return 1.0;
        };
        if over.is_zero() {
            return 0.0;
        }
        let done = started.elapsed().as_secs_f32() / over.as_secs_f32();
        (1.0 - done).clamp(0.0, 1.0)
    }

    /// Sets the open output's music gain after the volume or mute changed.
    fn apply_gain(&self) {
        if let Some(sink) = &self.sink {
            sink.input.set_gain(self.gain(sink.exclusive));
        }
    }

    /// Whether the open output, if any, can take a track of `info` as it is.
    fn output_fits(&self, info: &StreamInfo) -> bool {
        self.sink
            .as_ref()
            .is_none_or(|sink| sink.fits(&source_format(info)))
    }

    /// Close an output that cannot take a track of `info`, so that the next one opens at its
    /// format. Only for when nothing is queued: what the output still holds is cut off.
    fn close_unfit_output(&mut self, info: &StreamInfo) -> bool {
        if self.output_fits(info) {
            return false;
        }
        tracing::debug!(
            rate = info.sample_rate,
            bits = ?info.bits_per_sample,
            "reopening the output for a new format"
        );
        self.close_music_output();
        self.pending.clear();
        self.pending_from = 0;
        true
    }

    /// Samples heard so far. With no output, the next one's index space starts at zero.
    fn consumed(&self) -> u64 {
        self.sink.as_ref().map_or(0, |sink| sink.input.consumed())
    }

    /// The index the next converted sample will get.
    fn queued_index(&self) -> u64 {
        let written = self.sink.as_ref().map_or(0, |sink| sink.input.written());
        written + (self.pending.len() - self.pending_from) as u64
    }

    fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
        // A fade ends with the pause it led to; Play starts at the level again.
        if self.fade.take().is_some() {
            self.apply_gain();
        }
        if let Some(sink) = &self.sink {
            sink.input.set_paused(!playing);
        }
        if let Some(copy) = &self.copy {
            copy.set_paused(!playing);
        }
    }

    /// Throw away all audio not yet heard, the copy's included.
    fn discard_queued(&mut self) {
        self.pending.clear();
        self.pending_from = 0;
        if let Some(sink) = &mut self.sink {
            sink.input.clear();
        }
        if let Some(copy) = &mut self.copy {
            copy.clear();
        }
        if let Some(backlog) = &mut self.copy_backlog {
            backlog.clear();
        }
    }

    /// Let go of every track, and of the audio queued for them.
    fn drop_tracks(&mut self) {
        self.next = None;
        self.waiting = None;
        self.feed = None;
        self.segments.clear();
        self.discard_queued();
    }

    fn load(&mut self, track: TrackRequest, start_ms: u64, play: bool) {
        self.drop_tracks();
        self.set_playing(play);
        match TrackDecoder::open(&track.path) {
            Ok(decoder) => {
                self.close_unfit_output(decoder.info());
                let frame = scale(start_ms, decoder.info().sample_rate, 1_000);
                self.start_at(track, decoder, frame, false);
            }
            Err(error) => return self.fail(track.token, &error),
        }
        if play && self.has_track() {
            self.ensure_output();
        }
    }

    fn stop(&mut self) {
        let had_track = self.has_track();
        self.drop_tracks();
        self.set_playing(false);
        self.reading = ClockReading::default();
        if had_track {
            self.emit(EngineEvent::Stopped);
        }
    }

    /// Give up on everything because of `token`, and wait for a command.
    fn fail(&mut self, token: u64, error: &AudioError) {
        tracing::debug!(token, %error, "track failed");
        self.drop_tracks();
        self.set_playing(false);
        self.emit(EngineEvent::TrackFailed { token });
    }

    /// Make `decoder` the feed, from `frame` of its file. Past the end, the track is over at once:
    /// it is placed on its last frame with no audio left to play, so that it ends even while
    /// paused.
    fn start_at(
        &mut self,
        track: TrackRequest,
        mut decoder: TrackDecoder,
        frame: u64,
        started: bool,
    ) {
        let last_frame = decoder
            .info()
            .total_frames
            .map(|total| total.saturating_sub(1));
        let past_end = last_frame.filter(|last| frame > 0 && frame >= *last);
        let reached = match past_end {
            Some(last) => last,
            None if frame == 0 => 0,
            None => match decoder.seek(frame) {
                Ok(reached) => reached,
                Err(error) => return self.fail(track.token, &error),
            },
        };
        let index = self.queued_index();
        let gain = track.gain;
        let mut segment = Segment::new(track, decoder.info(), index, reached, started);
        if past_end.is_some() {
            segment.end_index = Some(index);
        } else {
            self.feed = Some(Feed::new(decoder, gain));
        }
        self.segments.push_back(segment);
    }

    /// Where the ear is now.
    fn heard(&self) -> Option<Heard> {
        let segment = self.segments.front()?;
        Some(Heard {
            track: segment.track.clone(),
            frame: segment.source_frame(self.consumed(), self.format()),
            started: segment.started,
        })
    }

    /// Throw away the queued audio and decode again from `from`. A track already queued behind
    /// the one being heard goes back to being the next one.
    fn restart(&mut self, from: Heard) {
        // The decoder can be reused if it is on this track. Frame 0 needs no seek on a new one.
        let on_this_track = self.segments.len() == 1 && from.frame > 0;
        let decoder = self.feed.take().filter(|_| on_this_track);
        if let Some(behind) = self.segments.drain(..).nth(1) {
            self.next = Some(behind.track);
        }
        if let Some((waiting, _)) = self.waiting.take() {
            self.next = Some(waiting);
        }
        self.discard_queued();
        let decoder = match decoder {
            Some(feed) => feed.decoder,
            None => match TrackDecoder::open(&from.track.path) {
                Ok(decoder) => decoder,
                Err(error) => return self.fail(from.track.token, &error),
            },
        };
        self.start_at(from.track, decoder, from.frame, from.started);
    }

    fn seek(&mut self, position_ms: u64) {
        let Some(segment) = self.segments.front() else {
            return;
        };
        let target = Heard {
            track: segment.track.clone(),
            frame: scale(position_ms, segment.rate, 1_000),
            started: segment.started,
        };
        self.restart(target);
    }

    fn set_next(&mut self, next: Option<TrackRequest>) {
        if let Some((waiting, _)) = &mut self.waiting {
            match next {
                // The same file under a new token: it is already open.
                Some(next) if next.path == waiting.path => *waiting = next,
                // Nothing of it is queued, so it can simply be let go.
                next => {
                    self.waiting = None;
                    self.next = next;
                }
            }
            return;
        }
        // The old next track could not be opened, and now nobody is waiting for it.
        if self.segments.len() > 1 && self.segments.back().is_some_and(Segment::is_unplayable) {
            self.segments.pop_back();
        }
        match (self.segments.get_mut(1), next) {
            // The same file under a new token: the queued audio is already right.
            (Some(queued), Some(next)) if queued.track.path == next.path => queued.track = next,
            // Something else was queued. Only decoding again can take it back out of the ring.
            (Some(_), next) => {
                if let Some(heard) = self.heard() {
                    self.restart(heard);
                }
                self.next = next;
            }
            (None, next) => self.next = next,
        }
    }

    fn set_device(&mut self, device: Option<DeviceId>) {
        self.device = device;
        let second_is_music = self.second.is_some()
            && self
                .sound_device
                .as_ref()
                .is_some_and(|second| self.is_music_device(second));
        if second_is_music {
            // Sounds for the second device now play on the music's output, once.
            self.close_second_output();
        }
        if self.sink.is_none() {
            return;
        }
        if !self.has_track() {
            // The next track opens the new device when it needs one.
            self.close_music_output();
            return;
        }
        match self.replace_output(true) {
            Some(true) => self.emit(EngineEvent::DeviceFellBack),
            Some(false) => {}
            None => self.lose_output(),
        }
    }

    /// Turn exclusive mode on or off. An open output in the other mode is replaced, and the track
    /// carries on from where the ear was.
    fn set_exclusive(&mut self, exclusive: bool) {
        if exclusive == self.exclusive {
            return;
        }
        self.exclusive = exclusive;
        if exclusive {
            // Whoever listens has taken the path for the system mixer since the mode went off,
            // whether or not that was ever said. The first exclusive state must be sent.
            self.last_output = None;
        }
        let Some(sink) = &self.sink else {
            return;
        };
        if sink.exclusive == exclusive {
            // Already right, as a shared output is when exclusive mode was refused.
            return;
        }
        if !self.has_track() {
            // Let go of the device. The next track opens an output in the new mode.
            self.close_music_output();
            return;
        }
        // The device was not changed, so landing on the default is no news.
        if self.replace_output(true).is_none() {
            self.lose_output();
        }
    }

    /// Open an output if there is none, as a first use or a retry after giving up. Landing on the
    /// default here is no news: the setting is kept, and the next open tries the device again.
    fn ensure_output(&mut self) {
        if self.sink.is_some() {
            return;
        }
        self.rebuilds.clear();
        if self.open_output().is_none() {
            self.lose_output();
        }
    }

    /// Open the configured device, then the system default, for the track being heard. Returns
    /// whether it had to fall back, or `None` if there is no output.
    fn open_output(&mut self) -> Option<bool> {
        let source = self.segments.front().and_then(|segment| segment.source);
        let request = OutputRequest {
            exclusive: self.exclusive,
            source,
        };
        let (opened, fell_back) = match self.factory.open(self.device.as_ref(), request) {
            Ok(output) => (Ok(output), false),
            Err(error) if self.device.is_some() => {
                tracing::debug!(%error, "could not open the chosen output");
                (self.factory.open(None, request), true)
            }
            Err(error) => (Err(error), false),
        };
        let sink = opened.and_then(|opened| {
            let Opened {
                mut output,
                exclusive,
                device,
            } = opened;
            let format = output.format();
            // One second of audio.
            let input = output.add_input(format.sample_rate as usize)?;
            input.set_gain(self.gain(exclusive));
            input.set_paused(!self.playing);
            Ok(Sink {
                input,
                spares: Spares::default(),
                output,
                format,
                exclusive,
                device,
                opened_for: source,
                fell_back,
            })
        });
        match sink {
            Ok(sink) => {
                self.share_dirty = true;
                if self.exclusive && !sink.exclusive {
                    tracing::debug!("exclusive mode is not available; the output is shared");
                    self.emit(EngineEvent::ExclusiveUnavailable);
                }
                self.sink = Some(sink);
                let second_is_music = self
                    .sound_device
                    .as_ref()
                    .is_some_and(|second| self.is_music_device(second));
                if fell_back && self.second.is_some() && second_is_music {
                    // The music fell back onto the second device. Sounds for it now play on the
                    // music's output, and the copy and the microphone stop, as when the music is
                    // moved there.
                    self.close_second_output();
                }
                Some(fell_back)
            }
            Err(error) => {
                tracing::debug!(%error, "could not open an output");
                None
            }
        }
    }

    /// Close the output and, if `open`, open another, then carry on from where the ear was.
    /// Returns what [`open_output`](Self::open_output) did.
    fn replace_output(&mut self, open: bool) -> Option<bool> {
        let heard = self.heard();
        self.close_music_output();
        let opened = if open { self.open_output() } else { None };
        if let Some(heard) = heard {
            self.restart(heard);
        }
        opened
    }

    fn lose_output(&mut self) {
        // Whoever listens forgets the path when nothing is open, so the next output's state is
        // sent even when it is the one before.
        if self.exclusive {
            self.last_output = None;
        }
        self.set_playing(false);
        self.emit(EngineEvent::NoDevice);
    }

    /// Replace an output that has failed, unless failures keep coming. A failed second output is
    /// let go of, and opened again by the next sound for it.
    fn check_output(&mut self) {
        if self
            .second
            .as_ref()
            .is_some_and(|second| second.output.failed())
        {
            tracing::debug!("the second output failed");
            if self.mic_opening.take().is_some() {
                // Opened again with the second output, as an open one is.
                self.mic_lost_with_second = true;
            }
            if self.mic.take().is_some() {
                // Tried again when the second output opens again.
                self.mic_lost_with_second = true;
                self.emit(EngineEvent::MicrophoneFailed { denied: false });
            }
            self.close_second_output();
            self.emit(EngineEvent::SoundDeviceFailed);
        }
        if !self.sink.as_ref().is_some_and(|sink| sink.output.failed()) {
            return;
        }
        if !self.has_track() {
            // Nothing is lost. The next track opens an output when it needs one.
            self.close_music_output();
            return;
        }
        let now = Instant::now();
        self.rebuilds
            .retain(|at| now.duration_since(*at) < REBUILD_WINDOW);
        let allowed = self.rebuilds.len() < MAX_REBUILDS;
        if allowed {
            self.rebuilds.push_back(now);
        }
        match self.replace_output(allowed) {
            Some(_) => self.emit(EngineEvent::DeviceFellBack),
            None => self.lose_output(),
        }
    }

    /// Decode while the ring has room.
    fn fill(&mut self) {
        while self.write_pending() {
            if self.feed.is_some() {
                self.decode();
            } else if self.gapless
                && self.next.is_some()
                && self.segments.back().is_some_and(|s| !s.failed)
            {
                // The next track was announced after the last one ran out. After a failure,
                // what plays next is for the next command to say.
                self.end_of_file();
            } else {
                return;
            }
        }
    }

    /// Move pending audio into the ring. Returns whether there is room for more.
    fn write_pending(&mut self) -> bool {
        let Some(sink) = &mut self.sink else {
            return false;
        };
        // Audio after a clear waits until the ring is empty, and then goes in all at once.
        if sink.input.clearing() {
            return false;
        }
        let written = sink.input.write(&self.pending[self.pending_from..]);
        // The copy gets what the listener's output took, and nothing it has not.
        let took = &self.pending[self.pending_from..self.pending_from + written];
        if let Some(copy) = &mut self.copy {
            copy.push(took);
        } else if self.share_music {
            let backlog = self.copy_backlog.get_or_insert_with(Vec::new);
            backlog.extend_from_slice(took);
            let format = sink.format;
            let most = BACKLOG.as_millis() as usize * format.sample_rate as usize / 1_000
                * usize::from(format.channels.max(1));
            // Trimmed now and then rather than on every write, which would move it all each time.
            if backlog.len() > 2 * most {
                let excess = backlog.len() - most;
                backlog.drain(..excess);
            }
        }
        self.pending_from += written;
        if self.pending_from < self.pending.len() {
            return false;
        }
        self.pending.clear();
        self.pending_from = 0;
        sink.input.free() > 0
    }

    fn decode(&mut self) {
        let (Some(feed), Some(format)) = (&mut self.feed, self.sink.as_ref().map(|s| s.format))
        else {
            return;
        };
        match feed.read(format, &mut self.mixed, &mut self.pending) {
            Ok(true) => {}
            Ok(false) => self.end_of_file(),
            Err(error) => self.feed_failed(&error),
        }
    }

    /// The feed has run out, or there was none. Move straight on to the next track if gapless.
    fn end_of_file(&mut self) {
        let next = if self.gapless { self.next.take() } else { None };
        let Some(track) = next else {
            return self.close_feed();
        };
        match TrackDecoder::open(&track.path) {
            Ok(decoder) => self.join(track, decoder),
            Err(error) => {
                tracing::debug!(token = track.token, %error, "next track failed");
                self.close_feed();
                let index = self.queued_index();
                self.segments.push_back(Segment::unplayable(track, index));
            }
        }
    }

    /// Flush the feed and mark where its track ends.
    fn close_feed(&mut self) {
        let Some(mut feed) = self.feed.take() else {
            return;
        };
        if let Some(resampler) = &mut feed.resampler
            && let Err(error) = resampler.flush(&mut self.pending)
        {
            tracing::debug!(%error, "lost the end of a track");
        }
        self.end_at_queue();
    }

    /// Mark the last track as ending where the queued audio does. Nothing else may stay beyond
    /// that either: a join comes from the frames read, and if their audio was lost in the
    /// resampler, an end past it would never be reached.
    fn end_at_queue(&mut self) {
        let end = self.queued_index();
        for segment in &mut self.segments {
            segment.start_index = segment.start_index.min(end);
            segment.end_index = segment.end_index.map(|index| index.min(end));
        }
        if let Some(segment) = self.segments.back_mut() {
            segment.end_index = Some(end);
        }
    }

    /// Follow the feed with `track`, with no gap between them.
    fn join(&mut self, track: TrackRequest, decoder: TrackDecoder) {
        if !self.output_fits(decoder.info()) {
            // An exclusive output plays one rate, and depths up to its own. This track waits
            // until the last one has been heard out, and then starts on an output opened for it.
            self.close_feed();
            self.waiting = Some((track, decoder));
            return;
        }
        let rate = decoder.info().sample_rate;
        let same_rate = self
            .feed
            .as_ref()
            .is_some_and(|feed| feed.decoder.info().sample_rate == rate);
        let format = self.format();
        let (Some(feed), Some(format), Some(segment), true) =
            (&mut self.feed, format, self.segments.back_mut(), same_rate)
        else {
            self.close_feed();
            return self.start_at(track, decoder, 0, false);
        };
        // The resampler runs on, so that no filter edge lands on the join. Some of the last track
        // is still inside it, so the join comes from the frames read rather than from the index.
        let frames = scale(feed.frames_read, format.sample_rate, rate);
        let join = segment.start_index + frames * u64::from(format.channels);
        segment.end_index = Some(join);
        feed.gain = track_gain(track.gain);
        let next = Segment::new(track, decoder.info(), join, 0, false);
        feed.decoder = decoder;
        feed.frames_read = 0;
        self.segments.push_back(next);
    }

    /// The feed broke. What it delivered still plays; the failure is reported after that.
    fn feed_failed(&mut self, error: &AudioError) {
        tracing::debug!(%error, "decoding stopped");
        self.next = None;
        let read_nothing = self.feed.as_ref().is_some_and(|f| f.frames_read == 0);
        self.close_feed();
        if let Some(segment) = self.segments.back_mut() {
            segment.failed = true;
            // Rounding at a join can leave it a frame of the last track's tail.
            if read_nothing {
                segment.end_index = Some(segment.start_index);
            }
        }
    }

    /// Report what the ear has reached, and act on the last track running out.
    fn settle(&mut self) {
        let consumed = self.consumed();
        let format = self.format();
        let mut finished = None;
        while let Some(segment) = self.segments.front_mut() {
            let reached_end = segment.end_index.is_some_and(|end| consumed >= end);
            let has_audio = segment.end_index != Some(segment.start_index);
            let heard = has_audio && consumed > segment.start_index;
            if !segment.started && (heard || (reached_end && !segment.failed)) {
                segment.started = true;
                self.events.push(EngineEvent::TrackStarted {
                    token: segment.track.token,
                    duration_ms: segment.duration_ms,
                });
            }
            if !reached_end {
                break;
            }
            let token = segment.track.token;
            let failed = segment.failed;
            if !failed {
                // The clock keeps this after the track has gone from the list.
                self.reading.token = token;
                self.reading.position_ms = segment.position_ms(consumed, format);
            }
            self.events.push(if failed {
                EngineEvent::TrackFailed { token }
            } else {
                EngineEvent::TrackEnded { token }
            });
            self.segments.pop_front();
            finished = Some(failed);
        }
        if !self.has_track()
            && let Some(failed) = finished
        {
            self.after_last_track(failed);
        }
    }

    /// Nothing is left in the ring: start the next track, or stop.
    fn after_last_track(&mut self, failed: bool) {
        if failed {
            return self.set_playing(false);
        }
        let (track, decoder) = match self.waiting.take() {
            Some(waiting) => waiting,
            None => {
                let Some(track) = self.next.take() else {
                    self.set_playing(false);
                    // The copy plays out the little it still holds, which the second device
                    // has yet to play. Anything queued from now on waits, paused, for Play.
                    if let Some(copy) = &mut self.copy {
                        copy.finish();
                        copy.set_paused(false);
                    }
                    return self.emit(EngineEvent::Stopped);
                };
                match TrackDecoder::open(&track.path) {
                    Ok(decoder) => (track, decoder),
                    Err(error) => return self.fail(track.token, &error),
                }
            }
        };
        // Before the track takes its place, which counts from the start of the output.
        let reopen = self.close_unfit_output(decoder.info());
        self.start_at(track, decoder, 0, false);
        // Not through `ensure_output`, which would forget the recent failures: a new format is
        // no fresh start.
        if reopen && self.playing && self.open_output().is_none() {
            self.lose_output();
        }
    }

    /// Close the music's output. Sounds on it are cut off there.
    fn close_music_output(&mut self) {
        self.sink = None;
        // The copy holds audio the listener's output had queued and has now lost. It starts
        // again with the next output.
        self.copy = None;
        self.copy_backlog = None;
        self.cut_sounds(Target::Music);
    }

    /// Close the second output, or stop opening it. Sounds on it are cut off there; sounds that
    /// waited for it are sent on again, as the second device now allows.
    fn close_second_output(&mut self) {
        self.close_microphone();
        self.copy = None;
        self.copy_backlog = None;
        // One being opened is let go of: when it is ready, nothing waits, and it closes there.
        self.second_opening = None;
        if let Some(second) = self.second.take() {
            let_go(second.output);
        }
        self.second_idle_since = None;
        self.cut_sounds(Target::Second);
        for waiting in std::mem::take(&mut self.waiting_sounds) {
            self.route_sound(waiting);
        }
    }

    /// Take sounds off `target`, whose output has closed. A sound left with no output has ended.
    fn cut_sounds(&mut self, target: Target) {
        let mut ended = Vec::new();
        self.voices.retain_mut(|voice| {
            voice.leave(target);
            if !voice.has_parts() {
                ended.push(voice.token);
            }
            voice.has_parts()
        });
        for token in ended {
            self.emit(EngineEvent::SoundEnded { token });
        }
    }

    /// Whether anything needs the second output now: a sound on it, the microphone, open or
    /// opening, or the music being copied while it plays.
    fn second_needed(&self) -> bool {
        self.sound_on(Target::Second)
            || self.mic.is_some()
            // One still opening will need it in a moment.
            || self.mic_opening.is_some()
            || (self.copy.is_some() && self.playing && self.has_track())
    }

    fn set_microphone(&mut self, on: bool, device: Option<DeviceId>, gain: f32) {
        self.mic_gain = mic_gain(gain);
        let moved = self
            .mic
            .as_ref()
            .is_some_and(|mic| mic.device() != device.as_ref());
        if self.mic_device != device {
            // One being opened is for the microphone chosen before.
            self.mic_opening = None;
        }
        self.mic_on = on;
        self.mic_device = device;
        self.mic_dirty = true;
        self.mic_lost_with_second = false;
        if !on || moved {
            self.close_microphone();
        } else if let Some(mic) = &self.mic {
            mic.set_gain(self.mic_gain);
        }
    }

    /// Close the microphone, if it is open.
    fn close_microphone(&mut self) {
        // One being opened is let go of: when it is ready, nothing waits, and it closes again.
        self.mic_opening = None;
        if self.mic.take().is_some() {
            self.emit(EngineEvent::MicrophoneClosed);
        }
    }

    /// Open or close the microphone, as the settings and the outputs now allow.
    fn sync_microphone(&mut self) {
        if self.mic.as_ref().is_some_and(Microphone::failed) {
            tracing::debug!("the microphone was lost");
            self.mic = None;
            self.emit(EngineEvent::MicrophoneFailed { denied: false });
        }
        if !std::mem::take(&mut self.mic_dirty)
            || !self.mic_on
            || self.mic.is_some()
            || self.mic_opening.is_some()
        {
            return;
        }
        if !self.second_device_is_another() {
            return;
        }
        match self.want_second_output() {
            SecondReady::Open => self.begin_microphone(),
            // Tried again once it is open.
            SecondReady::Opening => self.mic_dirty = true,
            SecondReady::Unavailable => {}
        }
    }

    /// Opens the microphone on a thread of its own, since a slow or stuck device can take
    /// seconds, and the music must not wait for it. The result comes back with
    /// [`finish_microphone`](Self::finish_microphone), which looks for it every tick.
    fn begin_microphone(&mut self) {
        let (opened_tx, opened) = mpsc::channel();
        let capture = Arc::clone(&self.capture);
        let device = self.mic_device.clone();
        let spawned = thread::Builder::new()
            .name("aria-audio-mic-open".into())
            .spawn(move || {
                let result = capture.open(device.as_ref());
                // Without a receiver, the engine has let this one go: the reader drops here, and
                // the microphone closes again.
                let _ = opened_tx.send(result);
            });
        match spawned {
            Ok(_) => self.mic_opening = Some(opened),
            Err(error) => {
                tracing::debug!(%error, "could not start opening the microphone");
                self.emit(EngineEvent::MicrophoneFailed { denied: false });
            }
        }
    }

    /// Takes in a microphone that has finished opening, if it is still wanted as it was asked
    /// for. One that is not is dropped, which closes it.
    fn finish_microphone(&mut self) {
        let Some(result) = self
            .mic_opening
            .as_ref()
            .and_then(|opened| opened.try_recv().ok())
        else {
            return;
        };
        self.mic_opening = None;
        if !self.mic_on || self.mic.is_some() || !self.second_device_is_another() {
            return;
        }
        // The second output is kept while the microphone opens; one that failed meanwhile brings
        // the microphone back when it opens again.
        let Some(second) = &mut self.second else {
            return;
        };
        let opened = result.and_then(|reader| {
            let name = reader.name().map(str::to_owned);
            let input = second
                .output
                .add_input(Microphone::ring_frames(second.format))?;
            Microphone::new(
                reader,
                input,
                second.format,
                self.mic_gain,
                self.mic_device.clone(),
            )
            .map(|mic| (mic, name))
        });
        match opened {
            Ok((mic, name)) => {
                self.mic = Some(mic);
                self.emit(EngineEvent::MicrophoneOpened { name });
            }
            Err(error) => {
                tracing::debug!(%error, "the microphone could not be opened");
                self.emit(match error {
                    AudioError::CableAsMicrophone => EngineEvent::MicrophoneIsCable,
                    AudioError::PermissionDenied => EngineEvent::MicrophoneFailed { denied: true },
                    _ => EngineEvent::MicrophoneFailed { denied: false },
                });
            }
        }
    }

    /// Move what the microphone has captured into the second output.
    fn feed_microphone(&mut self) {
        if let Some(mic) = &mut self.mic {
            mic.feed();
        }
    }

    fn set_share_music(&mut self, on: bool, gain: f32) {
        self.share_music = on;
        self.share_gain = copy_gain(gain);
        self.share_dirty = true;
        if !on {
            self.copy = None;
            self.copy_backlog = None;
        } else if let Some(copy) = &self.copy {
            copy.set_gain(self.share_gain);
        }
    }

    /// Start or end the copy of the music, as the settings and the outputs now allow.
    fn sync_copy(&mut self) {
        let format = self.format();
        let fits = self
            .copy
            .as_ref()
            .is_some_and(|copy| Some(copy.from()) == format);
        if !self.share_music || !fits {
            self.copy = None;
        }
        if !self.share_music {
            self.copy_backlog = None;
        }
        // A change is kept until a copy can be tried: with no output or no track, as after a
        // Stop, the next Load is when it is.
        let Some(from) = format else {
            return;
        };
        if !self.share_dirty || !self.share_music || self.copy.is_some() || !self.has_track() {
            return;
        }
        self.share_dirty = false;
        if !self.second_device_is_another() {
            return;
        }
        match self.want_second_output() {
            SecondReady::Open => {}
            SecondReady::Opening => {
                // Tried again once it is open, with what the listener takes meanwhile.
                self.share_dirty = true;
                return;
            }
            SecondReady::Unavailable => {
                self.copy_backlog = None;
                return;
            }
        }
        // What the listener's ring already holds is not in the copy. The copy starts with as
        // much silence, so the two play in step and the listener's output is left alone.
        let queued = self
            .sink
            .as_ref()
            // Audio just cleared, as at a load or a seek, is never heard.
            .filter(|sink| !sink.input.clearing())
            .map_or(0, |sink| {
                sink.input.written().saturating_sub(sink.input.consumed())
            });
        let (gain, paused) = (self.share_gain, !self.playing);
        let Some(second) = &mut self.second else {
            return;
        };
        // Two seconds: twice the listener's ring, so that only a second output that falls
        // behind ever drops any of the copy.
        let made = second
            .output
            .add_input(2 * second.format.sample_rate as usize)
            .and_then(|input| MusicCopy::new(input, from, second.format, gain, paused));
        let backlog = self.copy_backlog.take().unwrap_or_default();
        match made {
            Ok(mut copy) => {
                // What the listener has yet to hear and the backlog holds goes in as music; only
                // what is older than the backlog is silence.
                let held = (backlog.len() as u64).min(queued);
                copy.lead_in(queued - held);
                copy.push(&backlog[backlog.len() - held as usize..]);
                self.copy = Some(copy);
            }
            Err(error) => tracing::debug!(%error, "the music could not be copied"),
        }
        // Opening the second output just now is no change to try again for.
        self.share_dirty = false;
    }

    /// Write what the copy still holds, as after a clear.
    fn flush_copy(&mut self) {
        if let Some(copy) = &mut self.copy {
            copy.flush();
        }
    }

    /// Whether a sound plays on `target`.
    fn sound_on(&self, target: Target) -> bool {
        self.voices.iter().any(|voice| voice.plays_on(target))
    }

    fn play_sound(&mut self, token: u64, path: &Path, gain: f32, route: SoundRoute) {
        let decoder = match TrackDecoder::open(path) {
            Ok(decoder) => decoder,
            Err(error) => {
                tracing::debug!(token, %error, "sound failed");
                return self.emit(EngineEvent::SoundFailed { token });
            }
        };
        // Fired again while it waits, it waits as the newest.
        self.waiting_sounds.retain(|waiting| waiting.token != token);
        self.route_sound(WaitingSound {
            token,
            decoder,
            gain,
            route,
        });
    }

    /// Start a sound now, or, when the second output it needs is being opened, once that has
    /// finished, so that a sound for both devices starts on both together.
    fn route_sound(&mut self, sound: WaitingSound) {
        let to_second = sound.route != SoundRoute::Mine && self.second_device_is_another();
        let on_second = if to_second {
            match self.want_second_output() {
                SecondReady::Open => true,
                SecondReady::Opening => return self.waiting_sounds.push(sound),
                SecondReady::Unavailable => false,
            }
        } else {
            false
        };
        self.start_sound(sound, to_second, on_second);
    }

    /// Start a sound on the outputs its route and the second device allow: `to_second`, whether
    /// it is for the second device, and `on_second`, whether that is open.
    fn start_sound(&mut self, sound: WaitingSound, to_second: bool, on_second: bool) {
        let WaitingSound {
            token,
            decoder,
            gain,
            route,
        } = sound;
        let playing = self.voices.iter().position(|voice| voice.token == token);
        let on_music = (route != SoundRoute::Second || !to_second) && {
            self.ensure_output();
            self.sink.is_some()
        };
        if !on_music && !on_second {
            // Played nowhere. A sound already playing under the token plays on, and ends later.
            if playing.is_none() {
                self.emit(EngineEvent::SoundEnded { token });
            }
            return;
        }
        // Set up before anything else changes, so that a sound that cannot play leaves the others
        // as they were. Spares are taken first, so this needs a new input only when every input
        // on the output is in use: one more than the sounds playing at most.
        let mut voice = Voice::new(token, decoder, gain);
        if on_music && let Some(sink) = &mut self.sink {
            let added = voice.add_part(
                Target::Music,
                sink.output.as_mut(),
                sink.format,
                &mut sink.spares,
            );
            if let Err(error) = added {
                tracing::debug!(token, %error, "a sound could not join the music's output");
            }
        }
        if on_second && let Some(second) = &mut self.second {
            let added = voice.add_part(
                Target::Second,
                second.output.as_mut(),
                second.format,
                &mut second.spares,
            );
            if let Err(error) = added {
                tracing::debug!(token, %error, "a sound could not join the second output");
            }
        }
        if !voice.has_parts() {
            return self.emit(EngineEvent::SoundFailed { token });
        }
        // A sound fired again starts over, as the newest; a new one may push out the oldest.
        if let Some(at) = playing {
            if let Some(old) = self.voices.remove(at) {
                self.release(old);
            }
        } else if self.voices.len() >= SOUNDS_AT_ONCE
            && let Some(oldest) = self.voices.pop_front()
        {
            let oldest_token = oldest.token;
            self.release(oldest);
            self.emit(EngineEvent::SoundEnded {
                token: oldest_token,
            });
        }
        self.voices.push_back(voice);
        self.emit(EngineEvent::SoundStarted { token });
    }

    /// Whether there is a second device, and it is not the one the music plays on.
    fn second_device_is_another(&self) -> bool {
        let Some(second) = &self.sound_device else {
            return false;
        };
        // An open second output was another device when it was opened, and is checked again
        // whenever the music's device changes.
        self.second.is_some() || !self.is_music_device(second)
    }

    /// Whether `device` is the one the music plays on.
    ///
    /// With the music on the system default, chosen or fallen back to, this asks the factory for
    /// the default device. That happens for every sound for the second device while the second
    /// output is closed (so for every one while the second device is the default, or cannot be
    /// opened), and at every change of the music's device while it is open. The system factory
    /// answers with one query of the default endpoint rather than a listing of every device.
    fn is_music_device(&self, device: &DeviceId) -> bool {
        let on_default = match &self.device {
            Some(music) if music == device => return true,
            // The chosen device could not be opened, and the music plays on the default.
            Some(_) => self.sink.as_ref().is_some_and(|sink| sink.fell_back),
            None => true,
        };
        on_default && self.factory.default_device().as_ref() == Some(device)
    }

    /// Whether the second output is open, and if it is not, start opening it.
    fn want_second_output(&mut self) -> SecondReady {
        if self.second.is_some() {
            SecondReady::Open
        } else if self.second_opening.is_some() {
            SecondReady::Opening
        } else {
            self.begin_second_output()
        }
    }

    /// Opens the second output on a thread of its own: shared, whatever the music's mode. A
    /// device slow to open (Bluetooth, a sleeping USB interface) can take seconds, and the music
    /// must not wait for it. The result comes back with
    /// [`finish_second_output`](Self::finish_second_output), which looks for it every tick.
    fn begin_second_output(&mut self) -> SecondReady {
        let (opened_tx, opened) = mpsc::channel();
        let factory = Arc::clone(&self.factory);
        let device = self.sound_device.clone();
        let spawned = thread::Builder::new()
            .name("aria-audio-second-open".into())
            .spawn(move || {
                let request = OutputRequest {
                    exclusive: false,
                    source: None,
                };
                let result = factory.open(device.as_ref(), request);
                // Without a receiver, the engine has let this one go: it closes here.
                let _ = opened_tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.second_opening = Some(opened);
                SecondReady::Opening
            }
            Err(error) => {
                tracing::debug!(%error, "could not start opening the second output");
                self.emit(EngineEvent::SoundDeviceFailed);
                SecondReady::Unavailable
            }
        }
    }

    /// Takes in a second output that has finished opening, and starts the sounds that waited for
    /// it. One that could not be opened is reported once, and the sounds go where they can.
    fn finish_second_output(&mut self) {
        let Some(opening) = &self.second_opening else {
            return;
        };
        let result = match opening.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(AudioError::Device(
                "the second output's opener stopped".into(),
            )),
        };
        self.second_opening = None;
        let on_second = match result {
            Ok(opened) => {
                let format = opened.output.format();
                self.second = Some(SecondOutput {
                    output: opened.output,
                    format,
                    spares: Spares::default(),
                });
                // A copy that waited for the second output may start now, and a microphone that
                // went with the last one comes back.
                self.share_dirty = true;
                if std::mem::take(&mut self.mic_lost_with_second) {
                    self.mic_dirty = true;
                }
                true
            }
            Err(error) => {
                tracing::debug!(%error, "could not open the second output");
                self.emit(EngineEvent::SoundDeviceFailed);
                // Not tried again for the copy or the microphone until something changes.
                self.share_dirty = false;
                self.mic_dirty = false;
                false
            }
        };
        for waiting in std::mem::take(&mut self.waiting_sounds) {
            self.start_sound(waiting, true, on_second);
        }
    }

    /// Keep a voice's inputs for the sounds that come after it.
    fn release(&mut self, voice: Voice) {
        for (target, input) in voice.into_inputs() {
            match target {
                Target::Music => {
                    if let Some(sink) = &mut self.sink {
                        sink.spares.put(input);
                    }
                }
                Target::Second => {
                    if let Some(second) = &mut self.second {
                        second.spares.put(input);
                    }
                }
            }
        }
    }

    fn stop_sounds(&mut self) {
        for waiting in std::mem::take(&mut self.waiting_sounds) {
            self.emit(EngineEvent::SoundEnded {
                token: waiting.token,
            });
        }
        while let Some(voice) = self.voices.pop_front() {
            let token = voice.token;
            self.release(voice);
            self.emit(EngineEvent::SoundEnded { token });
        }
    }

    fn set_sound_device(&mut self, device: Option<DeviceId>) {
        if device != self.sound_device {
            self.sound_device = device;
            self.close_second_output();
        }
    }

    /// Decode sounds while their outputs have room.
    fn feed_sounds(&mut self) {
        for voice in &mut self.voices {
            voice.feed(&mut self.mixed);
        }
    }

    /// Report the sounds that have been heard to their end.
    fn settle_sounds(&mut self) {
        let mut index = 0;
        while index < self.voices.len() {
            if !self.voices[index].finished() {
                index += 1;
                continue;
            }
            if let Some(voice) = self.voices.remove(index) {
                let token = voice.token;
                self.release(voice);
                self.emit(EngineEvent::SoundEnded { token });
            }
        }
    }

    /// How the track being heard reaches the device, if there is an output and a track.
    fn output_state(&self) -> Option<OutputState> {
        let sink = self.sink.as_ref()?;
        let source = self.segments.front()?.source?;
        let channels = sink.device.channels;
        let facts = PathFacts {
            exclusive: sink.exclusive,
            source,
            device: sink.device,
            gain_is_unity: self.gain(sink.exclusive) == 1.0
                && self
                    .segments
                    .front()
                    .is_some_and(|segment| track_gain(segment.track.gain) == 1.0),
            // Mono to both channels counts as unchanged.
            channels_unchanged: source.channels == channels
                || (source.channels == 1 && channels == 2),
            sound_playing: self.sound_on(Target::Music),
        };
        Some(OutputState {
            path: output_path(facts),
            device_rate: sink.device.sample_rate,
            device_bits: sink.device.kind.bits(),
        })
    }

    /// Say how the sound reaches the device, when that has changed. With exclusive mode off this
    /// says nothing, except once after it is turned off, which is why the last state is kept only
    /// while the mode is on.
    fn report_output(&mut self) {
        if !self.exclusive && self.last_output.is_none() {
            return;
        }
        let Some(state) = self.output_state() else {
            return;
        };
        if self.last_output != Some(state) {
            self.emit(EngineEvent::Output(state));
        }
        self.last_output = self.exclusive.then_some(state);
    }

    fn publish(&mut self) {
        self.reading = match self.segments.front() {
            Some(segment) => ClockReading {
                token: segment.track.token,
                position_ms: segment.position_ms(self.consumed(), self.format()),
                playing: self.playing,
            },
            None => ClockReading {
                playing: false,
                ..self.reading
            },
        };
        self.clock.publish(self.reading);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeFactory;
    use EngineCommand::{Load, Pause, Play, Seek, SetNext, Stop};

    fn load(token: u64) -> EngineCommand {
        Load {
            track: request(token),
            start_ms: 0,
            play: true,
        }
    }

    fn request(token: u64) -> TrackRequest {
        TrackRequest {
            token,
            path: PathBuf::from("track"),
            gain: 1.0,
        }
    }

    #[test]
    fn only_the_last_seek_of_a_batch_counts() {
        let batch = vec![Seek(1), Seek(2), Pause, Seek(3), Play];

        assert_eq!(coalesce(batch), [Pause, Seek(3), Play]);
    }

    #[test]
    fn a_load_overrides_what_came_before_it() {
        let batch = vec![
            load(1),
            Seek(1),
            SetNext(None),
            load(2),
            Seek(2),
            SetNext(None),
        ];

        assert_eq!(coalesce(batch), [load(2), Seek(2), SetNext(None)]);
    }

    #[test]
    fn stop_overrides_what_came_before_it() {
        let batch = vec![load(1), Seek(1), SetNext(None), Stop, load(2)];

        assert_eq!(coalesce(batch), [Stop, load(2)]);
    }

    #[test]
    fn other_commands_all_stay_in_order() {
        let batch = vec![Play, Pause, Play];

        assert_eq!(coalesce(batch.clone()), batch);
    }

    #[test]
    fn no_segment_is_left_past_the_queued_audio() {
        let factory = FakeFactory::new(OutputFormat {
            sample_rate: 44_100,
            channels: 2,
        });
        let clock = Arc::new(PlaybackClock::default());
        let (microphone, _) = crate::fake::FakeCapture::new(crate::capture::CaptureFormat {
            sample_rate: 44_100,
            channels: 2,
        });
        let mut worker = Worker::new(
            Box::new(factory),
            Box::new(microphone),
            Box::new(|_| {}),
            clock,
            IDLE_GRACE,
        );
        // A join worked out from frames read, whose audio then never came out of the resampler.
        let mut first = Segment::unplayable(request(1), 0);
        first.end_index = Some(1_000);
        let second = Segment::unplayable(request(2), 1_000);
        worker.segments.extend([first, second]);

        worker.end_at_queue();

        let ends: Vec<_> = worker.segments.iter().map(|s| s.end_index).collect();
        let starts: Vec<_> = worker.segments.iter().map(|s| s.start_index).collect();
        assert_eq!(ends, [Some(0), Some(0)]);
        assert_eq!(starts, [0, 0]);
    }

    #[test]
    fn scale_keeps_huge_values_in_range() {
        assert_eq!(scale(10, 3, 2), 15);
        assert_eq!(scale(u64::MAX, 1_000, 1), u64::MAX);
        assert_eq!(scale(5, 1, 0), 5);
    }
}
