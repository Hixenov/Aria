//! The player engine: one thread that decodes tracks into an output and says what is heard.
//!
//! Audio runs ahead of the ear by up to one ring, so what the decoder has reached says little.
//! All writes to one input share a sample index, and a list of `Segment`s says which track each
//! stretch of it belongs to. `consumed()` then gives what is being heard, and that alone drives the
//! clock and the started and ended events.

use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::clock::{ClockReading, PlaybackClock};
use crate::decode::{StreamInfo, TrackDecoder};
use crate::device::DeviceId;
use crate::error::AudioError;
use crate::mixer::InputHandle;
use crate::output::{AudioOutput, OutputFactory, OutputFormat};
use crate::resample::{Resampler, convert_channels};

/// How often the thread looks at the output while a track is active.
const TICK: Duration = Duration::from_millis(10);
/// Rebuilding after an output failure stops at this many within [`REBUILD_WINDOW`]. Some errors
/// repeat for as long as a device is in a bad way, and each one fails the new output too.
const MAX_REBUILDS: usize = 3;
const REBUILD_WINDOW: Duration = Duration::from_secs(10);

/// One track to play. The token names it in events and on the clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackRequest {
    pub token: u64,
    pub path: PathBuf,
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
    SetVolume(f32),
    SetDevice(Option<DeviceId>),
    SetGapless(bool),
    /// Drop the track and what follows it.
    Stop,
}

/// What the engine reports. Tracks start and end when they are heard, not when they are decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// The output failed and was replaced, or the chosen device could not be opened.
    DeviceFellBack,
    /// No output could be opened, or outputs keep failing. Playback is paused, and the next
    /// [`Play`](EngineCommand::Play) tries again.
    NoDevice,
    /// The last track ended, or a [`Stop`](EngineCommand::Stop) took effect.
    Stopped,
}

/// Starts the engine thread.
pub struct Engine;

impl Engine {
    /// Start the thread. `on_event` runs on it, so it must not block.
    pub fn start(
        factory: Box<dyn OutputFactory>,
        on_event: Box<dyn Fn(EngineEvent) + Send>,
    ) -> io::Result<EngineHandle> {
        let (commands, received) = mpsc::channel();
        let (done_tx, done) = mpsc::sync_channel(1);
        let clock = Arc::new(PlaybackClock::default());
        let worker_clock = Arc::clone(&clock);
        let thread = thread::Builder::new()
            .name("aria-audio-engine".into())
            .spawn(move || {
                let _done = DoneGuard(done_tx);
                Worker::new(factory, on_event, worker_clock).run(&received);
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

/// Owned by the engine thread. Its drop signals [`EngineHandle::shutdown`], so a panicking thread
/// unblocks it too.
struct DoneGuard(SyncSender<()>);

impl Drop for DoneGuard {
    fn drop(&mut self) {
        // This is the only send on a capacity-1 channel, so it never blocks. It fails only if the
        // handle is gone and nobody is waiting.
        let _ = self.0.try_send(());
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
/// overrides what came before it. Besides saving work, this keeps two clears out of one fade, which
/// the mixer could only splice together at reduced level.
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

/// An open output and the one input the engine writes to.
struct Sink {
    input: InputHandle,
    output: Box<dyn AudioOutput>,
    format: OutputFormat,
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
    /// Built when there is an output, whose format it depends on.
    resampler: Option<Resampler>,
    /// Frames read from the decoder for the current segment.
    frames_read: u64,
}

impl Feed {
    fn new(decoder: TrackDecoder) -> Self {
        Self {
            decoder,
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
struct Worker {
    factory: Box<dyn OutputFactory>,
    on_event: Box<dyn Fn(EngineEvent) + Send>,
    clock: Arc<PlaybackClock>,
    device: Option<DeviceId>,
    volume: f32,
    gapless: bool,
    playing: bool,
    sink: Option<Sink>,
    feed: Option<Feed>,
    next: Option<TrackRequest>,
    segments: VecDeque<Segment>,
    /// Audio in the output's format still to go into the ring, from `pending_from` on.
    pending: Vec<f32>,
    pending_from: usize,
    /// Scratch space between the channel conversion and the resampler.
    mixed: Vec<f32>,
    /// When recent rebuilds after an output failure happened.
    rebuilds: VecDeque<Instant>,
    reading: ClockReading,
}

impl Worker {
    fn new(
        factory: Box<dyn OutputFactory>,
        on_event: Box<dyn Fn(EngineEvent) + Send>,
        clock: Arc<PlaybackClock>,
    ) -> Self {
        Self {
            factory,
            on_event,
            clock,
            device: None,
            volume: 1.0,
            gapless: true,
            playing: false,
            sink: None,
            feed: None,
            next: None,
            segments: VecDeque::new(),
            pending: Vec::new(),
            pending_from: 0,
            mixed: Vec::new(),
            rebuilds: VecDeque::new(),
            reading: ClockReading::default(),
        }
    }

    fn run(mut self, commands: &Receiver<EngineCommand>) {
        loop {
            // With an output and a track there is always something to watch. Otherwise nothing
            // can change until a command arrives.
            let first = if self.sink.is_some() && self.has_track() {
                match commands.recv_timeout(TICK) {
                    Ok(command) => Some(command),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match commands.recv() {
                    Ok(command) => Some(command),
                    Err(_) => return,
                }
            };
            let batch = first.into_iter().chain(commands.try_iter()).collect();
            for command in coalesce(batch) {
                self.handle(command);
            }
            self.check_output();
            self.fill();
            self.settle();
            self.publish();
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
                if self.has_track() {
                    self.set_playing(true);
                    self.ensure_output();
                }
            }
            EngineCommand::Pause => self.set_playing(false),
            EngineCommand::Seek(position_ms) => self.seek(position_ms),
            EngineCommand::SetVolume(volume) => {
                self.volume = volume;
                if let Some(sink) = &self.sink {
                    sink.input.set_gain(volume);
                }
            }
            EngineCommand::SetDevice(device) => self.set_device(device),
            EngineCommand::SetGapless(gapless) => self.gapless = gapless,
            EngineCommand::Stop => self.stop(),
        }
    }

    fn emit(&self, event: EngineEvent) {
        (self.on_event)(event);
    }

    fn has_track(&self) -> bool {
        !self.segments.is_empty()
    }

    fn format(&self) -> Option<OutputFormat> {
        self.sink.as_ref().map(|sink| sink.format)
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
        if let Some(sink) = &self.sink {
            sink.input.set_paused(!playing);
        }
    }

    /// Throw away all audio not yet heard.
    fn discard_queued(&mut self) {
        self.pending.clear();
        self.pending_from = 0;
        if let Some(sink) = &mut self.sink {
            sink.input.clear();
        }
    }

    /// Let go of every track, and of the audio queued for them.
    fn drop_tracks(&mut self) {
        self.next = None;
        self.feed = None;
        self.segments.clear();
        self.discard_queued();
    }

    fn load(&mut self, track: TrackRequest, start_ms: u64, play: bool) {
        self.drop_tracks();
        self.set_playing(play);
        match TrackDecoder::open(&track.path) {
            Ok(decoder) => {
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
        let mut segment = Segment::new(track, decoder.info(), index, reached, started);
        if past_end.is_some() {
            segment.end_index = Some(index);
        } else {
            self.feed = Some(Feed::new(decoder));
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
        if self.sink.is_none() {
            return;
        }
        if !self.has_track() {
            // The next track opens the new device when it needs one.
            self.sink = None;
            return;
        }
        match self.replace_output(true) {
            Some(true) => self.emit(EngineEvent::DeviceFellBack),
            Some(false) => {}
            None => self.lose_output(),
        }
    }

    /// Open an output if there is none, as a first use or a retry after giving up.
    fn ensure_output(&mut self) {
        if self.sink.is_some() {
            return;
        }
        self.rebuilds.clear();
        match self.open_output() {
            Some(true) => self.emit(EngineEvent::DeviceFellBack),
            Some(false) => {}
            None => self.lose_output(),
        }
    }

    /// Open the configured device, then the system default. Returns whether it had to fall back,
    /// or `None` if there is no output.
    fn open_output(&mut self) -> Option<bool> {
        let (opened, fell_back) = match self.factory.open(self.device.as_ref()) {
            Ok(output) => (Ok(output), false),
            Err(error) if self.device.is_some() => {
                tracing::debug!(%error, "could not open the chosen output");
                (self.factory.open(None), true)
            }
            Err(error) => (Err(error), false),
        };
        let sink = opened.and_then(|mut output| {
            let format = output.format();
            // One second of audio.
            let input = output.add_input(format.sample_rate as usize)?;
            input.set_gain(self.volume);
            input.set_paused(!self.playing);
            Ok(Sink {
                input,
                output,
                format,
            })
        });
        match sink {
            Ok(sink) => {
                self.sink = Some(sink);
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
        self.sink = None;
        let opened = if open { self.open_output() } else { None };
        if let Some(heard) = heard {
            self.restart(heard);
        }
        opened
    }

    fn lose_output(&mut self) {
        self.set_playing(false);
        self.emit(EngineEvent::NoDevice);
    }

    /// Replace an output that has failed, unless failures keep coming.
    fn check_output(&mut self) {
        if !self.sink.as_ref().is_some_and(|sink| sink.output.failed()) {
            return;
        }
        if !self.has_track() {
            // Nothing is lost. The next track opens an output when it needs one.
            self.sink = None;
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
            } else if self.gapless && self.next.is_some() && self.has_track() {
                // The next track was announced after the last one ran out.
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
        self.pending_from += sink.input.write(&self.pending[self.pending_from..]);
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
        let end = self.queued_index();
        if let Some(segment) = self.segments.back_mut() {
            segment.end_index = Some(end);
        }
    }

    /// Follow the feed with `track`, with no gap between them.
    fn join(&mut self, track: TrackRequest, decoder: TrackDecoder) {
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
        let next = Segment::new(track, decoder.info(), join, 0, false);
        feed.decoder = decoder;
        feed.frames_read = 0;
        self.segments.push_back(next);
    }

    /// The feed broke. What it delivered still plays; the failure is reported after that.
    fn feed_failed(&mut self, error: &AudioError) {
        tracing::debug!(%error, "decoding stopped");
        self.feed = None;
        self.next = None;
        let end = self.queued_index();
        if let Some(segment) = self.segments.back_mut() {
            segment.end_index = Some(end);
            segment.failed = true;
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
                (self.on_event)(EngineEvent::TrackStarted {
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
            (self.on_event)(if failed {
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
        let Some(track) = self.next.take() else {
            self.set_playing(false);
            return self.emit(EngineEvent::Stopped);
        };
        match TrackDecoder::open(&track.path) {
            Ok(decoder) => self.start_at(track, decoder, 0, false),
            Err(error) => self.fail(track.token, &error),
        }
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
    fn scale_keeps_huge_values_in_range() {
        assert_eq!(scale(10, 3, 2), 15);
        assert_eq!(scale(u64::MAX, 1_000, 1), u64::MAX);
        assert_eq!(scale(5, 1, 0), 5);
    }
}
