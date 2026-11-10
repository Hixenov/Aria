//! Playback: the queue, the engine and the library, tied together on the backend worker.
//!
//! The UI only sends [`Command`]s and reads [`Event`]s and the clock. Everything here runs on the
//! worker and never waits: engine commands are queued, and what is for the library or the UI
//! collects in two lists that the worker empties after each call.
//!
//! # Tokens
//!
//! The engine names tracks by token, and each `Load` and `SetNext` takes a new one, so an event can
//! always be told from an older one that crossed a command on its way. Three places hold the tokens
//! still alive: `current`, for the track being heard; `announced`, for the latest `SetNext`; and
//! `superseded`, for older announcements the engine may have reached before the newer one arrived.
//! A `Load` replaces them all, and an event for any other token is left over from before it.
//!
//! The queue moves on only when the engine says a track has started, because audio runs ahead of
//! the ear. Between `TrackEnded` for one track and `TrackStarted` for the next, the current track
//! has ended but its successor is not yet known, so nothing new is announced. If the track that
//! starts is not the one the queue would play, the queue changed while it was on its way. The queue
//! decides then, and a `Load` puts its choice on.

use std::{
    collections::VecDeque,
    mem,
    sync::Arc,
    time::{Duration, Instant},
};

use aria_audio::{DeviceId, EngineCommand, EngineEvent, EngineHandle, PlaybackClock, TrackRequest};
use aria_library::{LibrarySnapshot, QueueRecord, Track, TrackId};

use crate::{
    command::Command,
    event::Event,
    library_service::LibraryRequest,
    notice::{Notice, NoticeLevel},
    player::{PreviousAction, Queue, RepeatMode, UpNext},
    settings::Settings,
};

/// The shortest time between two saves of the queue.
pub const SAVE_INTERVAL: Duration = Duration::from_secs(5);
/// Playback stops at this many failures in a row, so a dead drive does not walk the whole queue.
pub const MAX_CONSECUTIVE_FAILURES: u32 = 5;
/// How long a track must play to end a run of failures.
pub const FAILURE_RESET_MS: u64 = 1_000;
/// How long a track must play for a play to be recorded. A shorter track counts when it ends.
pub const PLAY_COUNTS_AFTER_MS: u64 = 30_000;
/// How many upcoming tracks a [`PlayerView`] holds.
pub const UPCOMING_LIMIT: usize = 200;
/// How many replaced announcements are kept.
///
/// Only a `SetNext` replaces one, and the engine takes each command within milliseconds, so an
/// announcement older than the newest few can only still be ahead of the engine if that many queue
/// changes came in those milliseconds. Should one be dropped and then start anyway, its start is
/// ignored: the service still waits on the track before it, so it announces nothing, the engine
/// plays the dropped track to its end, and the queue then moves on from the track before it, which
/// plays the dropped track a second time. Eight changes within a few milliseconds take more than a
/// person can do, so eight is enough.
const SUPERSEDED_LIMIT: usize = 8;

/// Playback stopped after [`MAX_CONSECUTIVE_FAILURES`].
const STOPPED_AFTER_FAILURES: Notice =
    Notice::new(NoticeLevel::Error, "playback-stopped-after-failures");
/// The output failed and another one took over.
const DEVICE_FALLBACK: Notice = Notice::new(NoticeLevel::Warning, "playback-device-fallback");
/// No output could be opened.
const NO_DEVICE: Notice = Notice::new(NoticeLevel::Error, "playback-no-device");

/// What the UI shows of the player. The position comes from the clock.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlayerView {
    /// The current track. Its row may be gone from the library.
    pub current: Option<TrackId>,
    /// The engine's token for the current track, or 0 when none is loaded. The clock's position
    /// belongs to the current track only when its token is this one.
    pub token: u64,
    pub playing: bool,
    /// The length of the current track, or 0 while it is not known.
    pub duration_ms: u64,
    /// What comes next, at most [`UPCOMING_LIMIT`] tracks.
    pub upcoming: Vec<UpNext>,
}

/// A track handed to the engine, under the token that names it there.
#[derive(Debug, Clone)]
struct Given {
    token: u64,
    track: TrackId,
    /// Kept for a notice, because the row can go while the engine still holds the track.
    title: String,
}

/// How far restoring the saved queue has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Restore {
    /// It starts with the first snapshot, which the saved ids are checked against.
    AwaitingSnapshot,
    /// `LoadQueue` has gone out.
    AwaitingQueue,
    /// It happened, or something took its place.
    Done,
}

/// A track put in place by the restore at startup, and where it was left.
#[derive(Clone, Copy)]
struct Restored {
    token: u64,
    track: TrackId,
    position_ms: u64,
}

/// The synchronous state machine behind playback. The worker owns it.
pub struct PlaybackService {
    engine: EngineHandle,
    clock: Arc<PlaybackClock>,
    queue: Queue,
    /// `None` until the first one arrives.
    snapshot: Option<Arc<LibrarySnapshot>>,
    last_token: u64,
    /// What the engine holds as the track being heard. `None` means nothing is loaded.
    current: Option<Given>,
    /// `current` has ended, and what comes after it is not settled yet.
    ended: bool,
    /// What the engine was last told follows `current`.
    announced: Option<Given>,
    /// Earlier announcements, oldest first.
    superseded: VecDeque<Given>,
    /// Whether sound is wanted. The clock says what the engine is doing.
    playing: bool,
    /// Where `current` began, or was last moved to.
    played_from: u64,
    /// The length the engine reported for `current`, or 0.
    reported_ms: u64,
    /// Failures in a row.
    failures: u32,
    /// How long `current` must play to count as played: [`PLAY_COUNTS_AFTER_MS`], which the tests
    /// lower.
    play_counts_after_ms: u64,
    /// How long `current` played before `played_from`, when a seek moved it.
    heard_ms: u64,
    /// A play of `current` has been recorded.
    play_recorded: bool,
    /// Whether plays are recorded, as the settings last said.
    history: bool,
    restore: Restore,
    /// The track `restore_queue` loaded paused, while no user action has loaded anything since.
    restored: Option<Restored>,
    /// The restored track the engine could not open: it stays current, with its saved position,
    /// and Play tries it again.
    unplayable_restore: Option<Restored>,
    /// Play was pressed with nothing to play while the restore was still on its way.
    play_after_restore: bool,
    /// The lists of the queue (context, user queue, original order, seed, index) changed since
    /// they were last saved or restored.
    lists_dirty: bool,
    /// The current track and position last saved, or found on restoring.
    saved_position: Option<(Option<TrackId>, u64)>,
    last_save: Option<Instant>,
    /// The engine thread has died, so nothing will ever play.
    engine_gone: bool,
    // What the settings last said. These start as the engine and the queue do.
    volume: f32,
    device: Option<DeviceId>,
    gapless: bool,
    shuffle: bool,
    repeat: RepeatMode,
    /// For the UI, until the worker takes them.
    events: Vec<Event>,
    /// For the library, until the worker takes them.
    requests: Vec<LibraryRequest>,
}

impl PlaybackService {
    /// A service with an empty queue, on `engine`. `seed` seeds the shuffle.
    pub fn new(engine: EngineHandle, seed: u64) -> Self {
        let clock = engine.clock();
        Self {
            engine,
            clock,
            queue: Queue::new(seed),
            snapshot: None,
            last_token: 0,
            current: None,
            ended: false,
            announced: None,
            superseded: VecDeque::new(),
            playing: false,
            played_from: 0,
            reported_ms: 0,
            failures: 0,
            play_counts_after_ms: PLAY_COUNTS_AFTER_MS,
            heard_ms: 0,
            play_recorded: false,
            history: true,
            restored: None,
            unplayable_restore: None,
            restore: Restore::AwaitingSnapshot,
            play_after_restore: false,
            lists_dirty: false,
            saved_position: None,
            last_save: None,
            engine_gone: false,
            volume: 1.0,
            device: None,
            gapless: true,
            shuffle: false,
            repeat: RepeatMode::Off,
            events: Vec::new(),
            requests: Vec::new(),
        }
    }

    /// Takes what has collected for the UI.
    pub fn take_events(&mut self) -> Vec<Event> {
        mem::take(&mut self.events)
    }

    /// Takes what has collected for the library.
    pub fn take_requests(&mut self) -> Vec<LibraryRequest> {
        mem::take(&mut self.requests)
    }

    /// Carries out a playback command. Others are the worker's, and do nothing here.
    pub fn handle(&mut self, command: Command) {
        match command {
            Command::PlayContext { ids, start } => self.play_context(ids, start),
            Command::PlayNext(id) => {
                if self.exists(id) {
                    self.queue.play_next(id);
                    self.queue_changed();
                }
            }
            Command::AddToQueue(id) => {
                if self.exists(id) {
                    self.queue.add_to_queue(id);
                    self.queue_changed();
                }
            }
            Command::RemoveFromQueue(index) => {
                self.queue.remove_user(index);
                self.queue_changed();
            }
            Command::MoveInQueue { from, to } => {
                self.queue.move_user(from, to);
                self.queue_changed();
            }
            Command::ClearQueue => {
                self.queue.clear_user();
                self.queue_changed();
            }
            Command::TogglePlay => self.toggle_play(),
            Command::Next => self.next(),
            Command::Previous => self.previous(),
            Command::Seek(position_ms) => self.seek(position_ms),
            Command::UpdateSettings(_)
            | Command::ResetSettings
            | Command::AddFolder(_)
            | Command::RemoveFolder(_)
            | Command::Rescan
            | Command::ClearArtwork
            | Command::SetFavorite { .. }
            | Command::CreatePlaylist { .. }
            | Command::RenamePlaylist { .. }
            | Command::DeletePlaylist(_)
            | Command::AddToPlaylist { .. }
            | Command::RemoveFromPlaylist { .. }
            | Command::MoveInPlaylist { .. }
            | Command::ClearHistory
            | Command::ListDevices
            | Command::Shutdown => {}
        }
    }

    /// Acts on what the engine reports.
    pub fn on_engine(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::TrackStarted { token, duration_ms } => self.started(token, duration_ms),
            EngineEvent::TrackEnded { token } => {
                if self.current.as_ref().is_some_and(|c| c.token == token) {
                    self.ended = true;
                    // A track can end between two ticks, so its play counts here too.
                    if self.duration_ms().saturating_sub(self.played_from) >= FAILURE_RESET_MS {
                        self.failures = 0;
                    }
                    // A track shorter than the threshold counts once it has played in full.
                    self.record_play();
                    // While paused the engine would hold the next track without starting it.
                    if !self.playing {
                        self.move_on();
                    }
                } else {
                    tracing::debug!(token, "ignoring the end of a track no longer known");
                }
            }
            EngineEvent::TrackFailed { token } => self.failed(token),
            EngineEvent::DeviceFellBack => self.notify(DEVICE_FALLBACK),
            EngineEvent::NoDevice => {
                self.playing = false;
                self.notify(NO_DEVICE);
                self.publish();
            }
            EngineEvent::Stopped => self.stopped(),
        }
    }

    /// Takes in a new snapshot. The queue loses what is gone, but the playing track plays on even
    /// if its row is among that. The first snapshot starts the restore.
    pub fn on_snapshot(&mut self, snapshot: Arc<LibrarySnapshot>) {
        self.queue.retain(|id| snapshot.track(id).is_some());
        self.lists_dirty = true;
        self.snapshot = Some(snapshot);
        if self.restore == Restore::AwaitingSnapshot {
            self.restore = Restore::AwaitingQueue;
            self.requests.push(LibraryRequest::LoadQueue);
        }
        self.sync_next();
        self.publish();
    }

    /// Takes in the answer to `LoadQueue`, and restores what it holds, paused, keeping tracks added
    /// to the user queue in the meantime. If a pressed play got there first, the answer is dropped.
    pub fn on_queue_loaded(&mut self, record: Option<QueueRecord>) {
        if self.restore != Restore::AwaitingQueue {
            tracing::debug!("ignoring a saved queue that is no longer wanted");
            return;
        }
        self.restore = Restore::Done;
        // Until now nothing can have set a context, so this is all user queue.
        let added = !self.queue.upcoming(1).is_empty();
        if let Some(record) = record {
            self.restore_queue(record);
        }
        // What was restored is what is saved, apart from additions made while it was on its way.
        self.lists_dirty = added;
        self.saved_position = Some((self.queue.current(), self.position()));
        if mem::take(&mut self.play_after_restore) {
            self.toggle_play();
        }
    }

    /// The library thread has died. Nothing will answer it any more, so no restore is waited for,
    /// and the queue goes the way of an empty snapshot, which is what the UI now shows.
    pub fn library_lost(&mut self) {
        self.restore = Restore::Done;
        self.play_after_restore = false;
        self.on_snapshot(Arc::default());
    }

    /// Applies the settings that concern playback. Only what changed goes on, because setting a
    /// device, even the same one, rebuilds the output.
    pub fn on_settings(&mut self, settings: &Settings) {
        self.history = settings.history_enabled;
        let volume = if settings.muted { 0.0 } else { settings.volume };
        if volume != self.volume {
            self.volume = volume;
            self.engine.send(EngineCommand::SetVolume(volume));
        }
        if settings.output_device != self.device {
            self.device.clone_from(&settings.output_device);
            let device = settings.output_device.clone();
            self.engine.send(EngineCommand::SetDevice(device));
        }
        if settings.gapless != self.gapless {
            self.gapless = settings.gapless;
            self.engine
                .send(EngineCommand::SetGapless(settings.gapless));
        }
        if settings.shuffle != self.shuffle || settings.repeat != self.repeat {
            self.shuffle = settings.shuffle;
            self.repeat = settings.repeat;
            self.queue.set_shuffle(settings.shuffle);
            self.queue.set_repeat(settings.repeat);
            self.queue_changed();
        }
    }

    /// The engine thread has died. Nothing can play from now on, and the view says so.
    pub fn engine_lost(&mut self) {
        self.engine_gone = true;
        self.playing = false;
        self.notify(NO_DEVICE);
        self.publish();
    }

    /// The worker's regular call: ends a run of failures once a track has played long enough,
    /// records a play once it has played for the threshold, and saves what changed in the queue if
    /// a save is due.
    pub fn tick(&mut self) {
        self.tick_at(Instant::now());
    }

    fn tick_at(&mut self, now: Instant) {
        let reading = self.clock.read();
        if self
            .current
            .as_ref()
            .is_some_and(|c| c.token == reading.token)
            && reading.position_ms.saturating_sub(self.played_from) >= FAILURE_RESET_MS
        {
            self.failures = 0;
        }
        if self.played_ms() >= self.play_counts_after_ms {
            self.record_play();
        }
        let due = self
            .last_save
            .is_none_or(|at| now.duration_since(at) >= SAVE_INTERVAL);
        if due && self.restore == Restore::Done && self.save_pending() {
            self.last_save = Some(now);
        }
    }

    /// The last thing before shutdown: saves what is pending and silences the engine. The queue is
    /// left alone if it was never restored, as it would only overwrite the saved one with nothing.
    pub fn finish(&mut self) {
        if self.restore == Restore::Done {
            self.save_pending();
        }
        self.engine.send(EngineCommand::Stop);
    }

    /// Gives up the engine, for the worker to stop.
    pub fn into_engine(self) -> EngineHandle {
        self.engine
    }

    fn row(&self, id: TrackId) -> Option<&Track> {
        self.snapshot.as_ref()?.track(id)
    }

    fn exists(&self, id: TrackId) -> bool {
        self.row(id).is_some()
    }

    fn notify(&mut self, notice: Notice) {
        self.events.push(Event::Notice(notice));
    }

    /// The length of the current track: the library's, or else the engine's, or else 0.
    fn duration_ms(&self) -> u64 {
        let stored = self
            .queue
            .current()
            .and_then(|id| self.row(id))
            .map_or(0, |row| row.duration_ms);
        if stored > 0 { stored } else { self.reported_ms }
    }

    fn publish(&mut self) {
        let view = PlayerView {
            current: self.queue.current(),
            token: self.current.as_ref().map_or(0, |given| given.token),
            playing: self.playing,
            duration_ms: self.duration_ms(),
            upcoming: self.queue.upcoming(UPCOMING_LIMIT),
        };
        self.events.push(Event::Player(view));
    }

    fn queue_changed(&mut self) {
        self.lists_dirty = true;
        self.sync_next();
        self.publish();
    }

    /// Where the current track is. The clock lags a command, so until it names the track, this is
    /// where the track was asked to be.
    fn position(&self) -> u64 {
        let reading = self.clock.read();
        match &self.current {
            Some(current) if current.token == reading.token => reading.position_ms,
            Some(_) => self.played_from,
            None => self
                .unplayable_restore
                .filter(|restored| Some(restored.track) == self.queue.current())
                .map_or(0, |restored| restored.position_ms),
        }
    }

    /// How long the current track has played: pauses add nothing, and a seek adds nothing by
    /// itself. Until the clock names the track, it has played only what it did before a seek.
    fn played_ms(&self) -> u64 {
        let reading = self.clock.read();
        let since_seek = match &self.current {
            Some(current) if current.token == reading.token => {
                reading.position_ms.saturating_sub(self.played_from)
            }
            _ => 0,
        };
        self.heard_ms + since_seek
    }

    /// Records a play of the current track, once, if history is on.
    fn record_play(&mut self) {
        if !self.history || self.play_recorded {
            return;
        }
        if let Some(current) = &self.current {
            self.requests
                .push(LibraryRequest::RecordPlay(current.track));
            self.play_recorded = true;
        }
    }

    /// `current` has just become a track that starts at `start_ms`, with nothing played yet.
    fn start_counting(&mut self, start_ms: u64) {
        self.played_from = start_ms;
        self.heard_ms = 0;
        self.play_recorded = false;
    }

    /// Saves what changed since the last save: the whole queue if its lists did, else the position
    /// alone if that did, which costs the library one row. Returns whether anything went out.
    fn save_pending(&mut self) -> bool {
        let (current, position_ms) = (self.queue.current(), self.position());
        if mem::take(&mut self.lists_dirty) {
            let record = self.queue.to_record(position_ms);
            self.requests.push(LibraryRequest::SaveQueue(record));
        } else if self.saved_position != Some((current, position_ms)) {
            self.requests.push(LibraryRequest::SavePosition {
                current,
                position_ms,
            });
        } else {
            return false;
        }
        self.saved_position = Some((current, position_ms));
        true
    }

    /// Runs `step` on the queue. A track whose row is gone cannot be played again, so for it,
    /// repeat one counts as off.
    fn past_dead_repeat<T>(&mut self, step: impl FnOnce(&mut Queue) -> T) -> T {
        let dead = self.repeat == RepeatMode::One
            && self.queue.current().is_some_and(|id| !self.exists(id));
        if dead {
            self.queue.set_repeat(RepeatMode::Off);
        }
        let result = step(&mut self.queue);
        if dead {
            self.queue.set_repeat(RepeatMode::One);
        }
        result
    }

    /// Takes a new token for `track`, if its row is there.
    fn give(&mut self, track: TrackId) -> Option<(Given, TrackRequest)> {
        let row = self.row(track)?;
        let (title, path) = (row.title.clone(), row.path.clone());
        self.last_token += 1;
        let token = self.last_token;
        Some((
            Given {
                token,
                track,
                title,
            },
            TrackRequest { token, path },
        ))
    }

    /// Forgets every token. The engine holds none of them any more, or is about to.
    fn forget_tracks(&mut self) {
        self.current = None;
        self.ended = false;
        self.announced = None;
        self.superseded.clear();
        self.reported_ms = 0;
    }

    /// Replaces everything in the engine with `track`, and says what follows it.
    fn load(&mut self, track: TrackId, start_ms: u64, play: bool) {
        self.forget_tracks();
        self.restored = None;
        self.unplayable_restore = None;
        let Some((given, request)) = self.give(track) else {
            // Only a current track whose row has gone gets here.
            self.playing = false;
            self.engine.send(EngineCommand::Stop);
            return;
        };
        self.engine.send(EngineCommand::Load {
            track: request,
            start_ms,
            play,
        });
        self.current = Some(given);
        self.start_counting(start_ms);
        self.playing = play && !self.engine_gone;
        self.sync_next();
    }

    /// Tells the engine what follows the current track, if that is not what it was last told.
    /// With nothing loaded there is nothing to follow, and a track that has ended waits to see
    /// what starts after it.
    fn sync_next(&mut self) {
        if self.current.is_none() || self.ended {
            return;
        }
        let next = self.past_dead_repeat(|queue| queue.peek_after_current());
        if next == self.announced.as_ref().map(|given| given.track) {
            return;
        }
        if let Some(old) = self.announced.take() {
            self.superseded.push_back(old);
            if self.superseded.len() > SUPERSEDED_LIMIT {
                self.superseded.pop_front();
            }
        }
        let (announced, request) = next.and_then(|track| self.give(track)).unzip();
        self.announced = announced;
        self.engine.send(EngineCommand::SetNext(request));
    }

    /// Takes the announcement `token` names, which the engine has now reached. Those before it
    /// were passed over.
    fn take_announced(&mut self, token: u64) -> Option<Given> {
        if self.announced.as_ref().is_some_and(|a| a.token == token) {
            self.superseded.clear();
            return self.announced.take();
        }
        let index = self.superseded.iter().position(|s| s.token == token)?;
        self.superseded.drain(..=index).next_back()
    }

    /// Notes the length the engine reported for the current track, and has it stored if the
    /// library has none.
    fn learned_duration(&mut self, duration_ms: Option<u64>) {
        self.reported_ms = duration_ms.unwrap_or(0);
        let unknown = self.queue.current().filter(|id| {
            self.row(*id).is_some_and(|row| row.duration_ms == 0) && self.reported_ms > 0
        });
        if let Some(id) = unknown {
            let request = LibraryRequest::SetDuration(id, self.reported_ms);
            self.requests.push(request);
        }
    }

    fn started(&mut self, token: u64, duration_ms: Option<u64>) {
        if self.current.as_ref().is_some_and(|c| c.token == token) {
            self.learned_duration(duration_ms);
            return self.publish();
        }
        let Some(given) = self.take_announced(token) else {
            tracing::debug!(token, "ignoring the start of a track no longer known");
            return;
        };
        self.lists_dirty = true;
        match self.past_dead_repeat(Queue::advance_finished) {
            Some(track) if track == given.track => {
                self.current = Some(given);
                self.ended = false;
                self.start_counting(0);
                self.learned_duration(duration_ms);
                self.sync_next();
            }
            Some(track) => self.load(track, 0, self.playing),
            None => self.stop_at_end(),
        }
        self.publish();
    }

    /// The ring ran out. A next track the engine knew of would have started instead, so any
    /// announcement came too late for it, and only a `Load` can carry on.
    fn stopped(&mut self) {
        if self.ended {
            self.move_on();
        } else {
            tracing::debug!("ignoring a stop from before the last load");
        }
    }

    /// The current track has ended and no start will say what comes next, so the queue does.
    fn move_on(&mut self) {
        self.lists_dirty = true;
        match self.past_dead_repeat(Queue::advance_finished) {
            Some(track) => self.load(track, 0, self.playing),
            None => self.stop_at_end(),
        }
        self.publish();
    }

    /// Keep the last track at its start, ready to play again.
    fn stop_at_end(&mut self) {
        match self.queue.current() {
            Some(track) => self.load(track, 0, false),
            None => {
                self.forget_tracks();
                self.playing = false;
            }
        }
    }

    /// After a failure the engine holds, with nothing loaded and nothing to follow, so only a
    /// `Load` carries on. A failed track must not come round again, so this moves on as Next does.
    fn failed(&mut self, token: u64) {
        let was_current = self.current.as_ref().is_some_and(|c| c.token == token);
        let failed = if was_current {
            self.current.take()
        } else {
            self.take_announced(token)
        };
        let Some(failed) = failed else {
            tracing::debug!(token, "ignoring the failure of a track no longer known");
            return;
        };
        if let Some(restored) = self.restored.filter(|r| r.token == token && !self.playing) {
            // Nobody asked for this track, so the saved queue and position stay as they were.
            // Play tries it again, for the drive may be back by then.
            self.forget_tracks();
            self.restored = None;
            self.unplayable_restore = Some(restored);
            let notice = Notice::new(NoticeLevel::Warning, "playback-track-failed")
                .with_arg("title", failed.title);
            self.notify(notice);
            return self.publish();
        }
        self.lists_dirty = true;
        // An announced track is still ahead in the queue, unless the queue changed meanwhile.
        let replacement = if was_current {
            None
        } else {
            self.past_dead_repeat(Queue::advance_finished)
                .filter(|track| *track != failed.track)
        };
        self.forget_tracks();
        self.failures += 1;
        if self.failures >= MAX_CONSECUTIVE_FAILURES {
            self.playing = false;
            self.notify(STOPPED_AFTER_FAILURES);
            return self.publish();
        }
        let notice = Notice::new(NoticeLevel::Warning, "playback-track-failed")
            .with_arg("title", failed.title);
        self.notify(notice);
        match replacement.or_else(|| self.queue.advance_manual()) {
            Some(track) => self.load(track, 0, self.playing),
            None => self.playing = false,
        }
        self.publish();
    }

    fn restore_queue(&mut self, record: QueueRecord) {
        let position_ms = record.position_ms;
        let added: Vec<UpNext> = self.queue.upcoming(usize::MAX);
        let mut queue = Queue::from_record(record, self.shuffle, self.repeat, |id| self.exists(id));
        for item in added {
            queue.add_to_queue(item.track);
        }
        self.queue = queue;
        if let Some(track) = self.queue.current() {
            // The engine would end a track placed on its last frame at once.
            let stored = self.row(track).map_or(0, |row| row.duration_ms);
            let at_end = stored > 0 && position_ms >= stored;
            let position_ms = if at_end { 0 } else { position_ms };
            self.load(track, position_ms, false);
            self.restored = self.current.as_ref().map(|given| Restored {
                token: given.token,
                track,
                position_ms,
            });
        }
        self.publish();
    }

    fn play_context(&mut self, ids: Vec<TrackId>, start: usize) {
        if start >= ids.len() {
            return;
        }
        let dropped = ids.iter().take(start).filter(|id| !self.exists(**id));
        let start = start - dropped.count();
        let ids: Vec<TrackId> = ids.into_iter().filter(|id| self.exists(*id)).collect();
        // A chosen track that is gone gives way to what stood after it, or else before it.
        let start = start.min(ids.len().saturating_sub(1));
        if let Some(track) = self.queue.play_context(ids, start) {
            self.begin(track, true);
        }
    }

    /// Plays `track`, which the queue has just moved to. The user's choice replaces a restore that
    /// has not arrived yet.
    fn begin(&mut self, track: TrackId, play: bool) {
        self.restore = Restore::Done;
        self.play_after_restore = false;
        self.lists_dirty = true;
        self.load(track, 0, play);
        self.publish();
    }

    fn toggle_play(&mut self) {
        if self.engine_gone {
            self.notify(NO_DEVICE);
            return self.publish();
        }
        if self.current.is_some() {
            self.playing = !self.playing;
            self.engine.send(if self.playing {
                EngineCommand::Play
            } else {
                EngineCommand::Pause
            });
            return self.publish();
        }
        if let Some(restored) = self.unplayable_restore.filter(|restored| {
            Some(restored.track) == self.queue.current() && self.exists(restored.track)
        }) {
            self.load(restored.track, restored.position_ms, true);
            return self.publish();
        }
        match self.queue.advance_manual() {
            Some(track) => self.begin(track, true),
            None if self.restore != Restore::Done => {
                self.play_after_restore = !self.play_after_restore;
            }
            None => {}
        }
    }

    fn next(&mut self) {
        match self.queue.advance_manual() {
            Some(track) => self.begin(track, self.playing),
            None if self.current.is_some() => {
                self.stop_at_end();
                self.publish();
            }
            None => {}
        }
    }

    fn previous(&mut self) {
        match self.queue.previous(self.position()) {
            PreviousAction::Restart => self.seek(0),
            PreviousAction::Play(track) => self.begin(track, self.playing),
            PreviousAction::Nothing => {}
        }
    }

    fn seek(&mut self, position_ms: u64) {
        if self.current.is_some() {
            self.heard_ms += self.position().saturating_sub(self.played_from);
            self.played_from = position_ms;
            self.engine.send(EngineCommand::Seek(position_ms));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        path::PathBuf,
        sync::mpsc::{self, Receiver},
        thread,
    };

    use aria_audio::{
        ClockReading, Engine, OutputFormat,
        fake::{FakeControl, FakeFactory},
        testing::write_tone_wav,
    };
    use aria_library::{FolderId, Track};
    use tempfile::TempDir;

    use crate::{i18n::Localizer, player::RESTART_THRESHOLD_MS, settings::Language};

    use super::*;

    const FORMAT: OutputFormat = OutputFormat {
        sample_rate: 44_100,
        channels: 2,
    };
    /// Only a failing test waits this long.
    const DEADLINE: Duration = Duration::from_secs(20);
    const CHUNK_FRAMES: usize = 2_048;
    /// The least time between two pulls: a chunk at four times real speed. A real device takes
    /// audio as it plays it; a test that takes it any faster can run seconds ahead of the engine's
    /// clock on a busy machine (CI), and overshoot whatever it was waiting for.
    const PULL_SPACING: Duration =
        Duration::from_micros(CHUNK_FRAMES as u64 * 1_000_000 / FORMAT.sample_rate as u64 / 4);
    const SEED: u64 = 7;
    /// The play threshold in the tests of recording plays, which would each take some eight
    /// seconds at the real one. Their times are those of the rule scaled by the same factor: here
    /// 30 s is 1 000 ms, so a second is about 33 ms. A point "just under" the threshold is kept
    /// a few chunks under it, because the clock may run that far behind the test's pulls.
    const TEST_PLAY_MS: u64 = 1_000;

    fn t(n: i64) -> TrackId {
        TrackId(n)
    }

    /// A library in a temporary folder. Tracks get the ids 1, 2, 3 and so on, and the title
    /// `Track <id>`.
    struct Music {
        dir: TempDir,
        tracks: Vec<Track>,
    }

    impl Music {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
                tracks: Vec::new(),
            }
        }

        fn push(&mut self, path: PathBuf, duration_ms: u64) {
            let id = self.tracks.len() as i64 + 1;
            self.tracks.push(Track {
                id: t(id),
                folder: FolderId(1),
                path,
                title: format!("Track {id}"),
                artist: None,
                album: None,
                album_artist: None,
                track_no: None,
                disc_no: None,
                year: None,
                genre: None,
                duration_ms,
                sample_rate: 44_100,
                channels: 2,
                codec: Arc::from("pcm"),
                artwork: None,
                added_at: 0,
            });
        }

        fn file(&self) -> PathBuf {
            self.dir
                .path()
                .join(format!("{}.wav", self.tracks.len() + 1))
        }

        /// A tone of `ms` milliseconds.
        fn tone(mut self, ms: u64) -> Self {
            let path = self.file();
            write_tone_wav(&path, 44_100, 2, ms * 441 / 10, 440.0).unwrap();
            self.push(path, ms);
            self
        }

        /// A tone whose length the library does not know.
        fn untimed_tone(mut self, ms: u64) -> Self {
            let path = self.file();
            write_tone_wav(&path, 44_100, 2, ms * 441 / 10, 440.0).unwrap();
            self.push(path, 0);
            self
        }

        /// A row whose file is not there.
        fn missing(mut self) -> Self {
            let path = self.file();
            self.push(path, 1_000);
            self
        }

        fn path(&self, id: i64) -> &PathBuf {
            &self.tracks[id as usize - 1].path
        }

        fn snapshot(&self) -> Arc<LibrarySnapshot> {
            self.without(&[])
        }

        /// The library with the rows of `gone` removed.
        fn without(&self, gone: &[i64]) -> Arc<LibrarySnapshot> {
            let tracks = self
                .tracks
                .iter()
                .filter(|track| !gone.contains(&track.id.0))
                .cloned()
                .collect();
            Arc::new(LibrarySnapshot {
                tracks,
                ..LibrarySnapshot::default()
            })
        }
    }

    /// A service on an engine on a fake output, with everything it reported so far. Nothing plays
    /// until a test pulls frames, and nothing the engine reports reaches the service until a test
    /// pumps it, so a test can also hand the service events of its own.
    struct Rig {
        service: PlaybackService,
        factory: FakeFactory,
        engine_events: Receiver<EngineEvent>,
        notices: Vec<Notice>,
        views: Vec<PlayerView>,
        requests: Vec<LibraryRequest>,
        /// When the fake device last took a chunk.
        last_pull: Cell<Option<Instant>>,
    }

    impl Rig {
        /// A service that has seen no snapshot yet.
        fn idle() -> Self {
            let factory = FakeFactory::new(FORMAT);
            let (sender, engine_events) = mpsc::channel();
            let on_event = Box::new(move |event| {
                let _ = sender.send(event);
            });
            let engine = Engine::start(Box::new(factory.clone()), on_event).unwrap();
            Self {
                service: PlaybackService::new(engine, SEED),
                factory,
                engine_events,
                notices: Vec::new(),
                views: Vec::new(),
                requests: Vec::new(),
                last_pull: Cell::new(None),
            }
        }

        /// A service with `music` as its library and nothing saved to restore.
        fn with(music: &Music) -> Self {
            let mut rig = Self::idle();
            rig.service.on_snapshot(music.snapshot());
            rig.service.on_queue_loaded(None);
            rig.collect();
            rig.requests.clear();
            rig.views.clear();
            rig
        }

        /// Move what the service reported into the logs.
        fn collect(&mut self) {
            for event in self.service.take_events() {
                match event {
                    Event::Notice(notice) => self.notices.push(notice),
                    Event::Player(view) => self.views.push(view),
                    other => panic!("the service does not send {other:?}"),
                }
            }
            self.requests.extend(self.service.take_requests());
        }

        fn send(&mut self, command: Command) {
            self.service.handle(command);
            self.collect();
        }

        fn play(&mut self, ids: &[i64], start: usize) {
            self.send(Command::PlayContext {
                ids: ids.iter().copied().map(t).collect(),
                start,
            });
        }

        fn snapshot(&mut self, snapshot: Arc<LibrarySnapshot>) {
            self.service.on_snapshot(snapshot);
            self.collect();
        }

        fn restore(&mut self, record: Option<QueueRecord>) {
            self.service.on_queue_loaded(record);
            self.collect();
        }

        fn settings(&mut self, settings: &Settings) {
            self.service.on_settings(settings);
            self.collect();
        }

        /// Hand the service an event of the test's own.
        fn engine(&mut self, event: EngineEvent) {
            self.service.on_engine(event);
            self.collect();
        }

        fn started(&mut self, token: u64) {
            self.engine(EngineEvent::TrackStarted {
                token,
                duration_ms: None,
            });
        }

        fn ended(&mut self, token: u64) {
            self.engine(EngineEvent::TrackEnded { token });
        }

        /// Hand the service what the engine reported so far.
        fn pump(&mut self) {
            while let Ok(event) = self.engine_events.try_recv() {
                self.service.on_engine(event);
            }
            self.collect();
        }

        fn output(&self) -> Option<FakeControl> {
            self.factory.outputs().last().cloned()
        }

        fn pull(&self) -> Vec<f32> {
            if let Some(last) = self.last_pull.get() {
                thread::sleep(PULL_SPACING.saturating_sub(last.elapsed()));
            }
            self.last_pull.set(Some(Instant::now()));
            self.output()
                .map_or_else(Vec::new, |output| output.pull(CHUNK_FRAMES))
        }

        /// Pump, without playing anything, until `done` holds.
        fn wait_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
            let deadline = Instant::now() + DEADLINE;
            loop {
                self.pump();
                if done(self) {
                    return;
                }
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                thread::sleep(Duration::from_micros(200));
            }
        }

        /// Play one chunk of the current track and no more. Unlike [`run_until`](Self::run_until),
        /// this cannot overshoot: a pause that shows on the clock has seen the ring filled.
        fn play_a_little(&mut self) {
            let token = self.current_token();
            self.send(Command::TogglePlay);
            self.wait_until("the pause", |rig| {
                rig.reading().token == token && !rig.reading().playing
            });
            self.send(Command::TogglePlay);
            self.wait_until("playback", |rig| rig.reading().playing);
            self.pull();
            self.wait_until("the clock to move", heard(token));
        }

        /// Play and pump until `done` holds.
        fn run_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
            let deadline = Instant::now() + DEADLINE;
            loop {
                self.pull();
                self.pump();
                if done(self) {
                    return;
                }
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                thread::sleep(Duration::from_micros(200));
            }
        }

        fn reading(&self) -> ClockReading {
            self.service.clock.read()
        }

        fn view(&self) -> &PlayerView {
            self.views.last().unwrap()
        }

        fn notices(&self, key: &str) -> Vec<&Notice> {
            self.notices.iter().filter(|n| n.key == key).collect()
        }

        fn current_token(&self) -> u64 {
            self.service.current.as_ref().unwrap().token
        }

        fn current_track(&self) -> TrackId {
            self.service.current.as_ref().unwrap().track
        }

        fn announced_token(&self) -> u64 {
            self.service.announced.as_ref().unwrap().token
        }

        fn announced_track(&self) -> Option<TrackId> {
            self.service.announced.as_ref().map(|given| given.track)
        }

        fn upcoming(&self) -> Vec<TrackId> {
            self.view().upcoming.iter().map(|up| up.track).collect()
        }

        fn saved_positions(&self) -> Vec<(Option<TrackId>, u64)> {
            self.requests
                .iter()
                .filter_map(|request| match request {
                    LibraryRequest::SavePosition {
                        current,
                        position_ms,
                    } => Some((*current, *position_ms)),
                    _ => None,
                })
                .collect()
        }

        /// Counts a play after [`TEST_PLAY_MS`] instead of half a minute.
        fn counting_soon(mut self) -> Self {
            self.service.play_counts_after_ms = TEST_PLAY_MS;
            self
        }

        fn tick(&mut self) {
            self.service.tick();
            self.collect();
        }

        /// Play, pump and tick, as the worker would, until `done` holds.
        fn run_ticking_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
            let deadline = Instant::now() + DEADLINE;
            loop {
                self.pull();
                self.pump();
                self.tick();
                if done(self) {
                    return;
                }
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                thread::sleep(Duration::from_micros(200));
            }
        }

        fn position(&self) -> u64 {
            self.reading().position_ms
        }

        fn saved_records(&self) -> Vec<&QueueRecord> {
            self.requests
                .iter()
                .filter_map(|request| match request {
                    LibraryRequest::SaveQueue(record) => Some(record),
                    _ => None,
                })
                .collect()
        }
    }

    fn is_playing(track: i64) -> impl Fn(&Rig) -> bool {
        move |rig| {
            rig.views
                .last()
                .is_some_and(|view| view.current == Some(t(track)) && view.playing)
        }
    }

    fn shows(track: i64) -> impl Fn(&Rig) -> bool {
        move |rig| {
            rig.views
                .last()
                .is_some_and(|view| view.current == Some(t(track)))
        }
    }

    fn stopped(rig: &Rig) -> bool {
        rig.views.last().is_some_and(|view| !view.playing)
    }

    fn heard(token: u64) -> impl Fn(&Rig) -> bool {
        move |rig| rig.reading().token == token && rig.reading().position_ms > 0
    }

    /// The tracks recorded as played, in order.
    fn plays(rig: &Rig) -> Vec<TrackId> {
        rig.requests
            .iter()
            .filter_map(|request| match request {
                LibraryRequest::RecordPlay(track) => Some(*track),
                _ => None,
            })
            .collect()
    }

    /// A saved queue of `context`, with its first track current.
    fn record(context: &[i64]) -> QueueRecord {
        QueueRecord {
            context: context.iter().copied().map(t).collect(),
            current: context.first().copied().map(t),
            context_index: Some(0),
            ..QueueRecord::default()
        }
    }

    #[test]
    fn every_language_names_the_failed_track() {
        let notice = Notice::new(NoticeLevel::Warning, "playback-track-failed")
            .with_arg("title", "Blue in Green");
        for language in Language::ALL {
            let localizer = Localizer::new(language).unwrap();
            let text = localizer.tr_args(notice.key, &notice.fluent_args());
            assert!(text.contains("Blue in Green"), "{language:?}: {text}");
        }
    }

    // The tests the task names.

    #[test]
    fn play_context_starts_the_chosen_track() {
        let music = Music::new().tone(300).tone(3_000).tone(300);
        let mut rig = Rig::with(&music);

        rig.play(&[1, 2, 3], 1);

        assert_eq!(rig.view().current, Some(t(2)));
        assert!(rig.view().playing);
        assert_eq!(rig.view().duration_ms, 3_000);
        assert_eq!(rig.upcoming(), [t(3)]);
        rig.run_until("the track to be heard", heard(rig.current_token()));
    }

    #[test]
    fn the_view_carries_the_engine_token_of_a_loaded_track() {
        let music = Music::new().tone(200).tone(3_000);
        let mut rig = Rig::with(&music);
        assert_eq!(rig.service.clock.read().token, 0);

        rig.play(&[1, 2], 0);
        let first = rig.current_token();
        assert_ne!(first, 0);
        assert_eq!(rig.view().token, first);

        rig.send(Command::Next);
        let second = rig.current_token();
        assert_ne!(second, first);
        assert_eq!(rig.view().token, second, "a new load brings its token");
    }

    #[test]
    fn the_view_takes_the_announced_token_when_the_next_track_starts() {
        let music = Music::new().tone(200).tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2], 0);
        let first = rig.current_token();
        // The engine was told of the second track under this token, with no load.
        let announced = rig.announced_token();
        assert_ne!(announced, first);
        assert_eq!(rig.view().token, first);

        rig.run_until("the second track", is_playing(2));

        assert_eq!(rig.view().token, announced);
        assert_eq!(rig.current_token(), announced);
    }

    #[test]
    fn the_view_token_follows_a_failure_and_is_zero_when_nothing_is_loaded() {
        let music = Music::new().tone(200).missing().tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        rig.run_until("the third track", is_playing(3));
        assert_ne!(rig.view().token, 0);
        assert_eq!(rig.view().token, rig.current_token());

        let music = Music::new()
            .missing()
            .missing()
            .missing()
            .missing()
            .missing()
            .missing();
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3, 4, 5, 6], 0);
        rig.wait_until("the stop", |rig| {
            !rig.notices("playback-stopped-after-failures").is_empty()
        });
        assert_eq!(rig.view().token, 0, "nothing is loaded after the stop");
    }

    #[test]
    fn next_track_is_announced_for_gapless() {
        let music = Music::new().tone(200).tone(200);
        let mut rig = Rig::with(&music);

        rig.play(&[1, 2], 0);

        assert_eq!(rig.announced_track(), Some(t(2)));
        let next = rig.announced_token();
        assert_ne!(next, rig.current_token());
        // The engine knows it too: it plays on under that token with no further command.
        rig.run_until("the next track", |rig| rig.reading().token == next);
    }

    #[test]
    fn natural_advance_updates_the_view() {
        let music = Music::new().tone(200).tone(3_000).tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        assert_eq!(rig.upcoming(), [t(2), t(3)]);

        rig.run_until("the second track", is_playing(2));

        assert_eq!(rig.upcoming(), [t(3)]);
        assert_eq!(rig.announced_track(), Some(t(3)));
        rig.run_until("the third track", is_playing(3));
        assert!(rig.upcoming().is_empty());
        assert_eq!(rig.announced_track(), None);
    }

    #[test]
    fn previous_past_threshold_seeks_to_zero() {
        let music = Music::new().tone(200).tone(6_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2], 0);
        rig.send(Command::Next);
        let token = rig.current_token();
        rig.run_until("the threshold", |rig| {
            rig.reading().position_ms > RESTART_THRESHOLD_MS
        });

        rig.send(Command::Previous);

        assert_eq!(rig.current_token(), token, "the track is not reloaded");
        assert_eq!(rig.service.queue.current(), Some(t(2)));
        rig.wait_until("the seek", |rig| rig.reading().position_ms < 1_000);
        assert_eq!(rig.reading().token, token);
    }

    #[test]
    fn failed_track_is_skipped_with_a_notice() {
        let music = Music::new().tone(200).missing().tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);

        rig.run_until("the third track", is_playing(3));

        let notices = rig.notices("playback-track-failed");
        assert_eq!(notices.len(), 1, "{:?}", rig.notices);
        assert_eq!(notices[0].args, [("title", "Track 2".to_owned())]);
        rig.run_until("the track to be heard", heard(rig.current_token()));
    }

    #[test]
    fn five_failures_stop_playback() {
        let music = Music::new()
            .missing()
            .missing()
            .missing()
            .missing()
            .missing()
            .missing();
        let mut rig = Rig::with(&music);

        rig.play(&[1, 2, 3, 4, 5, 6], 0);
        rig.wait_until("the stop", |rig| {
            !rig.notices("playback-stopped-after-failures").is_empty()
        });

        let failed: Vec<_> = rig
            .notices("playback-track-failed")
            .iter()
            .map(|notice| notice.args[0].1.clone())
            .collect();
        assert_eq!(failed, ["Track 1", "Track 2", "Track 3", "Track 4"]);
        assert_eq!(rig.notices("playback-stopped-after-failures").len(), 1);
        assert_eq!(rig.view().current, Some(t(5)));
        assert_eq!(rig.upcoming(), [t(6)]);
        assert!(!rig.view().playing);
        assert!(rig.service.current.is_none(), "nothing is loaded");
    }

    #[test]
    fn failure_count_resets_after_a_second_of_play() {
        let music = Music::new().missing().missing().tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        rig.wait_until("the track that works", is_playing(3));
        rig.play_a_little();
        assert_eq!(rig.service.failures, 2);

        rig.service.tick();
        assert!(rig.reading().position_ms < FAILURE_RESET_MS);
        assert_eq!(rig.service.failures, 2);

        rig.run_until("a second of play", |rig| {
            rig.reading().position_ms >= FAILURE_RESET_MS
        });
        rig.service.tick();
        assert_eq!(rig.service.failures, 0);
    }

    #[test]
    fn queue_is_saved_and_restored_paused() {
        let music = Music::new().tone(3_000).tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2], 0);
        rig.run_until("some play", |rig| rig.reading().position_ms > 500);
        rig.send(Command::TogglePlay);
        let token = rig.current_token();
        rig.wait_until("the pause", |rig| {
            let reading = rig.reading();
            reading.token == token && !reading.playing
        });
        let position = rig.reading().position_ms;
        rig.service.tick();
        rig.collect();
        let saved = (*rig.saved_records().last().unwrap()).clone();
        assert_eq!(saved.current, Some(t(1)));
        assert_eq!(saved.position_ms, position);

        let mut restored = Rig::idle();
        restored.snapshot(music.snapshot());
        assert_eq!(restored.requests, [LibraryRequest::LoadQueue]);
        restored.restore(Some(saved));

        assert_eq!(restored.view().current, Some(t(1)));
        assert!(!restored.view().playing);
        assert_eq!(restored.upcoming(), [t(2)]);
        let token = restored.current_token();
        restored.wait_until("the track to load", |rig| rig.reading().token == token);
        assert!(restored.reading().position_ms.abs_diff(position) <= 50);
        assert!(!restored.reading().playing);
        assert!(restored.factory.outputs().is_empty(), "an output opened");

        // Nothing was lost either: pressing play carries on from there.
        restored.send(Command::TogglePlay);
        assert!(restored.view().playing);
        restored.wait_until("an output", |rig| rig.output().is_some());
    }

    #[test]
    fn restore_drops_missing_tracks() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::idle();
        rig.snapshot(music.snapshot());

        rig.restore(Some(QueueRecord {
            user: vec![t(98)],
            ..record(&[1, 99, 2])
        }));

        assert_eq!(rig.view().current, Some(t(1)));
        assert_eq!(rig.upcoming(), [t(2)]);
    }

    #[test]
    fn snapshot_change_keeps_playing_and_drops_gone_tracks() {
        let music = Music::new().tone(3_000).tone(300).tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        let token = rig.current_token();
        rig.run_until("the track to be heard", heard(token));

        // The playing track and the next one both go.
        rig.snapshot(music.without(&[1, 2]));

        assert_eq!(rig.view().current, Some(t(1)));
        assert!(rig.view().playing);
        assert_eq!(rig.upcoming(), [t(3)]);
        assert_eq!(
            rig.announced_track(),
            Some(t(3)),
            "the next one is replaced"
        );
        assert_eq!(rig.current_token(), token, "the track is not reloaded");
        let position = rig.reading().position_ms;
        rig.run_until("more of it", |rig| rig.reading().position_ms > position);
        assert_eq!(rig.reading().token, token);
        rig.run_until("the third track", is_playing(3));
        assert_eq!(rig.factory.outputs().len(), 1);
    }

    #[test]
    fn muted_sends_zero_volume() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::with(&music);
        rig.settings(&Settings {
            muted: true,
            volume: 0.8,
            ..Settings::default()
        });
        rig.play(&[1], 0);
        rig.run_until("the track to be heard", |rig| {
            rig.reading().position_ms > 100
        });

        assert!(rig.pull().iter().all(|sample| *sample == 0.0));

        rig.settings(&Settings {
            muted: false,
            volume: 0.8,
            ..Settings::default()
        });
        rig.run_until("sound", |rig| rig.pull().iter().any(|s| *s != 0.0));
    }

    #[test]
    fn end_of_list_stops_on_last_track() {
        let music = Music::new().tone(200).tone(200);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2], 0);

        rig.run_until("the end of the list", stopped);

        assert_eq!(rig.view().current, Some(t(2)));
        assert!(rig.upcoming().is_empty());
        let token = rig.current_token();
        rig.wait_until("the track to load", |rig| rig.reading().token == token);
        assert_eq!(rig.reading().position_ms, 0);
        assert!(!rig.reading().playing);

        rig.send(Command::TogglePlay);
        rig.run_until("the last track again", heard(token));
    }

    // What follows the current track.

    #[test]
    fn next_is_announced_only_when_it_changes() {
        let music = Music::new().tone(300).tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        let announced = rig.announced_token();
        let tokens = rig.service.last_token;

        rig.send(Command::RemoveFromQueue(5));
        rig.send(Command::ClearQueue);
        rig.snapshot(music.snapshot());
        rig.settings(&Settings {
            repeat: RepeatMode::All,
            ..Settings::default()
        });

        assert_eq!(rig.announced_token(), announced);
        assert_eq!(rig.service.last_token, tokens, "a SetNext went out");

        rig.send(Command::PlayNext(t(3)));
        assert_eq!(rig.announced_track(), Some(t(3)));
        rig.send(Command::ClearQueue);
        assert_eq!(rig.announced_track(), Some(t(2)));
    }

    #[test]
    fn the_track_after_next_waits_for_the_next_to_start() {
        let music = Music::new().tone(300).tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        let (first, second) = (rig.current_token(), rig.announced_token());
        rig.started(first);
        let tokens = rig.service.last_token;

        rig.ended(first);
        rig.snapshot(music.snapshot());
        rig.send(Command::ClearQueue);
        assert_eq!(rig.service.last_token, tokens);
        assert_eq!(rig.announced_track(), Some(t(2)));

        rig.started(second);
        assert_eq!(rig.view().current, Some(t(2)));
        assert_eq!(rig.current_token(), second);
        assert_eq!(rig.announced_track(), Some(t(3)));
    }

    #[test]
    fn a_superseded_token_still_counts_as_the_next_track() {
        let music = Music::new().tone(300).tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        let (first, old) = (rig.current_token(), rig.announced_token());
        rig.started(first);
        // Away from track 2 and back: track 2 is announced again under a new token, while the
        // engine may already have started it under the old one.
        rig.send(Command::PlayNext(t(3)));
        rig.send(Command::RemoveFromQueue(0));
        assert_eq!(rig.announced_track(), Some(t(2)));
        assert_ne!(rig.announced_token(), old);

        rig.ended(first);
        rig.started(old);

        assert_eq!(rig.view().current, Some(t(2)));
        assert_eq!(rig.current_token(), old);
        assert_eq!(rig.upcoming(), [t(3)]);
        // Otherwise the engine would play track 2 once more.
        assert_eq!(rig.announced_track(), Some(t(3)));
    }

    #[test]
    fn a_track_replaced_too_late_gives_way_to_the_queue() {
        let music = Music::new().tone(300).tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        let (first, old) = (rig.current_token(), rig.announced_token());
        rig.started(first);
        rig.send(Command::PlayNext(t(3)));

        // The engine had already moved on to track 2.
        rig.ended(first);
        rig.started(old);

        assert_eq!(rig.view().current, Some(t(3)));
        assert_eq!(rig.current_track(), t(3));
        assert_ne!(rig.current_token(), old);
        assert_eq!(rig.upcoming(), [t(2), t(3)], "track 2 keeps its place");
    }

    #[test]
    fn tokens_stay_bounded() {
        let music = Music::new().tone(300).tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);

        for _ in 0..100 {
            rig.send(Command::PlayNext(t(3)));
            rig.send(Command::RemoveFromQueue(0));
        }
        assert_eq!(rig.service.superseded.len(), SUPERSEDED_LIMIT);

        let next = rig.announced_token();
        rig.ended(rig.current_token());
        rig.started(next);
        assert!(rig.service.superseded.is_empty());
        rig.send(Command::PlayNext(t(1)));
        assert!(!rig.service.superseded.is_empty());
        rig.send(Command::Next);
        assert!(rig.service.superseded.is_empty(), "a load replaces them");
    }

    #[test]
    fn tokens_no_longer_known_are_ignored() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2], 0);
        let old = rig.current_token();
        rig.send(Command::Next);
        let current = rig.current_token();
        rig.views.clear();

        rig.started(old);
        rig.ended(old);
        rig.engine(EngineEvent::TrackFailed { token: old });
        rig.started(9_999);
        rig.ended(9_999);
        rig.engine(EngineEvent::TrackFailed { token: 9_999 });

        assert_eq!(rig.current_token(), current);
        assert!(rig.views.is_empty(), "{:?}", rig.views);
        assert!(rig.notices.is_empty(), "{:?}", rig.notices);
    }

    #[test]
    fn a_stop_from_before_a_load_is_ignored() {
        let music = Music::new().tone(300).tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        let old = rig.current_token();
        rig.send(Command::Next);
        let current = rig.current_token();

        // The first track had already run out when the Load went off.
        rig.ended(old);
        rig.engine(EngineEvent::Stopped);

        assert_eq!(rig.current_token(), current);
        assert_eq!(rig.view().current, Some(t(2)));
        assert!(rig.view().playing);
    }

    #[test]
    fn a_next_track_announced_too_late_is_loaded() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1], 0);
        let first = rig.current_token();
        rig.started(first);
        rig.send(Command::AddToQueue(t(2)));
        let announced = rig.announced_token();

        // The ring had drained, so the engine ignored that SetNext.
        rig.ended(first);
        rig.engine(EngineEvent::Stopped);

        assert_eq!(rig.view().current, Some(t(2)));
        assert!(rig.view().playing);
        assert_ne!(rig.current_token(), announced);
        rig.run_until("the track", heard(rig.current_token()));
    }

    #[test]
    fn a_track_added_as_the_last_one_ends_is_played() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1], 0);
        let first = rig.current_token();
        rig.started(first);
        rig.ended(first);

        rig.send(Command::AddToQueue(t(2)));
        assert_eq!(rig.announced_track(), None);
        rig.engine(EngineEvent::Stopped);

        assert_eq!(rig.view().current, Some(t(2)));
        assert_eq!(rig.current_track(), t(2));
    }

    #[test]
    fn a_replaced_track_that_fails_is_not_walked_into() {
        let music = Music::new().tone(300).tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        let (first, old) = (rig.current_token(), rig.announced_token());
        rig.started(first);
        rig.send(Command::PlayNext(t(3)));

        rig.ended(first);
        rig.engine(EngineEvent::TrackFailed { token: old });

        assert_eq!(rig.notices("playback-track-failed").len(), 1);
        assert_eq!(rig.view().current, Some(t(3)));
        assert_eq!(rig.current_track(), t(3));
    }

    #[test]
    fn a_failure_mid_track_moves_on() {
        let music = Music::new().tone(300).tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        let first = rig.current_token();
        rig.started(first);

        // The engine holds after this, so only a Load carries on.
        rig.engine(EngineEvent::TrackFailed { token: first });

        assert_eq!(rig.notices[0].args, [("title", "Track 1".to_owned())]);
        assert_eq!(rig.view().current, Some(t(2)));
        assert_eq!(rig.current_track(), t(2));
        assert_eq!(rig.announced_track(), Some(t(3)));
    }

    #[test]
    fn repeat_one_plays_the_track_again_under_new_tokens() {
        let music = Music::new().tone(200).tone(200);
        let mut rig = Rig::with(&music);
        rig.settings(&Settings {
            repeat: RepeatMode::One,
            ..Settings::default()
        });
        rig.play(&[1, 2], 0);
        assert_eq!(rig.announced_track(), Some(t(1)));
        let (first, again) = (rig.current_token(), rig.announced_token());
        rig.started(first);
        rig.ended(first);

        rig.started(again);

        assert_eq!(rig.view().current, Some(t(1)));
        assert_eq!(rig.current_token(), again);
        assert_eq!(rig.announced_track(), Some(t(1)));
        assert_ne!(rig.announced_token(), again);
    }

    #[test]
    fn a_deleted_track_does_not_repeat_forever() {
        let music = Music::new().tone(200).tone(200);
        let mut rig = Rig::with(&music);
        rig.settings(&Settings {
            repeat: RepeatMode::One,
            ..Settings::default()
        });
        rig.play(&[1, 2], 0);

        rig.snapshot(music.without(&[1]));
        assert_eq!(rig.announced_track(), Some(t(2)));

        rig.run_until("the track after it", is_playing(2));
        assert_eq!(rig.announced_track(), Some(t(2)), "this one may repeat");
    }

    #[test]
    fn a_deleted_track_with_nothing_after_it_stops() {
        let music = Music::new().tone(200);
        let mut rig = Rig::with(&music);
        rig.settings(&Settings {
            repeat: RepeatMode::One,
            ..Settings::default()
        });
        rig.play(&[1], 0);

        rig.snapshot(music.without(&[1]));
        assert_eq!(rig.announced_track(), None);

        rig.run_until("the stop", stopped);
        assert!(rig.service.current.is_none(), "nothing is left to load");
        assert!(rig.notices.is_empty(), "{:?}", rig.notices);
    }

    #[test]
    fn a_failed_track_under_repeat_one_moves_on() {
        let music = Music::new().missing().tone(200);
        let mut rig = Rig::with(&music);
        rig.settings(&Settings {
            repeat: RepeatMode::One,
            ..Settings::default()
        });

        rig.play(&[1, 2], 0);
        rig.wait_until("the second track", shows(2));

        assert_eq!(rig.notices("playback-track-failed").len(), 1);
    }

    #[test]
    fn a_failure_at_the_end_of_the_list_stops_without_trying_again() {
        let music = Music::new().missing();
        let mut rig = Rig::with(&music);

        rig.play(&[1], 0);
        rig.wait_until("the failure", stopped);

        assert_eq!(rig.notices("playback-track-failed").len(), 1);
        assert!(rig.service.current.is_none());
        rig.send(Command::TogglePlay);
        assert!(rig.service.current.is_none(), "nothing follows it");
    }

    #[test]
    fn a_failed_restore_leaves_the_queue_and_the_saved_position_alone() {
        let music = Music::new().missing().tone(300);
        let mut rig = Rig::idle();
        rig.snapshot(music.snapshot());
        let mut saved = record(&[1, 2]);
        saved.position_ms = 567;
        rig.restore(Some(saved));

        rig.wait_until("the failure", |rig| {
            rig.service.unplayable_restore.is_some()
        });
        rig.service.tick_at(Instant::now() + SAVE_INTERVAL * 2);
        rig.service.tick_at(Instant::now() + SAVE_INTERVAL * 4);
        rig.pump();

        assert_eq!(rig.view().current, Some(t(1)));
        assert_eq!(rig.upcoming(), [t(2)]);
        assert!(!rig.view().playing);
        assert!(rig.factory.outputs().is_empty());
        assert_eq!(rig.notices("playback-track-failed").len(), 1);
        assert!(rig.notices("playback-stopped-after-failures").is_empty());
        assert_eq!(rig.service.position(), 567);
        assert!(
            !rig.requests.iter().any(|r| matches!(
                r,
                LibraryRequest::SaveQueue(_) | LibraryRequest::SavePosition { .. }
            )),
            "{:?}",
            rig.requests
        );
    }

    #[test]
    fn play_after_a_failed_restore_tries_the_same_track_again() {
        let music = Music::new().missing().tone(3_000);
        let mut rig = Rig::idle();
        rig.snapshot(music.snapshot());
        rig.restore(Some(record(&[1, 2])));
        rig.wait_until("the failure", |rig| {
            rig.service.unplayable_restore.is_some()
        });

        rig.send(Command::TogglePlay);

        assert_eq!(rig.view().current, Some(t(1)));
        assert!(rig.view().playing);
        rig.wait_until("the second try to fail", shows(2));
        assert_eq!(rig.notices("playback-track-failed").len(), 2);
        assert!(rig.view().playing, "after a user action it moves on");
    }

    #[test]
    fn a_file_deleted_while_announced_is_skipped() {
        let music = Music::new().tone(200).tone(200).tone(3_000);
        let mut rig = Rig::with(&music);
        rig.settings(&Settings {
            gapless: false,
            ..Settings::default()
        });
        rig.play(&[1, 2, 3], 0);
        std::fs::remove_file(music.path(2)).unwrap();

        rig.run_until("the third track", is_playing(3));

        assert_eq!(rig.notices("playback-track-failed").len(), 1);
        assert_eq!(rig.notices[0].args, [("title", "Track 2".to_owned())]);
    }

    #[test]
    fn a_short_track_played_in_full_ends_a_run_of_failures() {
        let music = Music::new().missing().missing().tone(1_500).tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3, 4], 0);

        // No tick at all: the track ends between two of them.
        rig.run_until("the track after it", |rig| {
            rig.service.queue.current() == Some(t(4))
        });

        assert_eq!(rig.notices("playback-track-failed").len(), 2);
        assert_eq!(rig.service.failures, 0);
    }

    #[test]
    fn a_track_under_a_second_does_not_end_a_run_of_failures() {
        let music = Music::new().missing().missing().tone(500).tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3, 4], 0);

        rig.run_until("the track after it", |rig| {
            rig.service.queue.current() == Some(t(4))
        });

        assert_eq!(rig.service.failures, 2);
    }

    // Commands.

    #[test]
    fn commands_before_the_first_snapshot_do_nothing() {
        let mut rig = Rig::idle();

        rig.play(&[1, 2], 0);
        rig.send(Command::PlayNext(t(1)));
        rig.send(Command::AddToQueue(t(2)));
        rig.send(Command::Next);
        rig.send(Command::Seek(1_000));

        assert!(rig.service.current.is_none());
        assert_eq!(rig.service.queue.current(), None);
        assert!(rig.service.queue.upcoming(10).is_empty());
    }

    #[test]
    fn ids_not_in_the_snapshot_are_dropped() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::with(&music);

        rig.play(&[50, 1, 51, 2], 3);
        assert_eq!(rig.view().current, Some(t(2)), "the chosen track starts");
        assert!(rig.upcoming().is_empty());

        rig.send(Command::AddToQueue(t(50)));
        rig.send(Command::PlayNext(t(51)));
        assert!(rig.service.queue.upcoming(10).is_empty());

        // The chosen track itself is gone: what stood after it starts.
        rig.play(&[1, 50, 2], 1);
        assert_eq!(rig.view().current, Some(t(2)));

        let tokens = rig.service.last_token;
        rig.play(&[50, 51], 0);
        rig.play(&[1, 2], 2);
        assert_eq!(rig.service.last_token, tokens, "nothing to play");
        assert_eq!(rig.view().current, Some(t(2)));
    }

    #[test]
    fn toggle_with_nothing_loaded_starts_what_comes_next() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::with(&music);

        rig.send(Command::TogglePlay);
        assert!(rig.service.current.is_none(), "there is nothing to play");

        rig.send(Command::AddToQueue(t(2)));
        assert!(rig.service.current.is_none());
        rig.send(Command::TogglePlay);

        assert_eq!(rig.view().current, Some(t(2)));
        assert!(rig.view().playing);
        assert_eq!(rig.current_track(), t(2));
    }

    #[test]
    fn toggle_pauses_and_plays() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1], 0);
        let token = rig.current_token();

        rig.send(Command::TogglePlay);
        assert!(!rig.view().playing);
        rig.wait_until("the pause", |rig| {
            rig.reading().token == token && !rig.reading().playing
        });

        rig.send(Command::TogglePlay);
        assert!(rig.view().playing);
        rig.wait_until("playback", |rig| rig.reading().playing);
        assert_eq!(rig.current_token(), token);
    }

    #[test]
    fn seek_with_nothing_loaded_does_nothing() {
        let music = Music::new().tone(300);
        let mut rig = Rig::with(&music);

        rig.send(Command::Seek(100));

        assert!(rig.views.is_empty());
        assert_eq!(rig.service.played_from, 0);
    }

    #[test]
    fn seek_moves_the_track_being_heard() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1], 0);
        rig.send(Command::TogglePlay);

        rig.send(Command::Seek(2_000));

        rig.wait_until("the seek", |rig| {
            rig.reading().position_ms.abs_diff(2_000) <= 50
        });
    }

    #[test]
    fn a_seek_past_the_end_while_paused_moves_on_paused() {
        let music = Music::new().tone(3_000).tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2], 0);
        rig.send(Command::TogglePlay);

        rig.send(Command::Seek(10_000_000));
        rig.wait_until("the next track", shows(2));

        assert!(!rig.view().playing);
        let token = rig.current_token();
        rig.wait_until("the load", |rig| rig.reading().token == token);
        assert!(!rig.reading().playing);
        rig.send(Command::TogglePlay);
        rig.run_until("the track", heard(token));
    }

    #[test]
    fn previous_with_no_track_does_nothing() {
        let music = Music::new().tone(300);
        let mut rig = Rig::with(&music);

        rig.send(Command::Previous);

        assert!(rig.views.is_empty());
        assert!(rig.service.current.is_none());
    }

    #[test]
    fn previous_within_the_threshold_goes_back() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2], 0);
        rig.send(Command::Next);
        assert_eq!(rig.view().current, Some(t(2)));

        rig.send(Command::Previous);

        assert_eq!(rig.view().current, Some(t(1)));
        assert_eq!(rig.current_track(), t(1));
        assert_eq!(rig.announced_track(), Some(t(2)));
    }

    #[test]
    fn next_at_the_end_of_the_list_stops_on_the_last_track() {
        let music = Music::new().tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1], 0);
        let token = rig.current_token();

        rig.send(Command::Next);

        assert_eq!(rig.view().current, Some(t(1)));
        assert!(!rig.view().playing);
        assert_ne!(rig.current_token(), token);
    }

    #[test]
    fn next_keeps_a_paused_player_paused() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2], 0);
        rig.send(Command::TogglePlay);

        rig.send(Command::Next);

        assert_eq!(rig.view().current, Some(t(2)));
        assert!(!rig.view().playing);
    }

    #[test]
    fn the_view_holds_at_most_the_limit() {
        let mut music = Music::new();
        for _ in 0..UPCOMING_LIMIT + 2 {
            music = music.missing();
        }
        let mut rig = Rig::with(&music);
        let ids: Vec<i64> = (1..=UPCOMING_LIMIT as i64 + 2).collect();

        rig.play(&ids, 0);

        assert_eq!(rig.view().upcoming.len(), UPCOMING_LIMIT);
    }

    // What the engine reports.

    #[test]
    fn a_learned_duration_is_stored_and_shown() {
        let music = Music::new().untimed_tone(200).untimed_tone(200).tone(200);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2, 3], 0);
        assert_eq!(rig.view().duration_ms, 0);
        let (first, second) = (rig.current_token(), rig.announced_token());

        rig.engine(EngineEvent::TrackStarted {
            token: first,
            duration_ms: Some(210),
        });
        assert_eq!(rig.view().current, Some(t(1)));
        assert_eq!(rig.view().duration_ms, 210);
        let durations = |rig: &Rig| -> Vec<LibraryRequest> {
            rig.requests
                .iter()
                .filter(|request| matches!(request, LibraryRequest::SetDuration(..)))
                .cloned()
                .collect()
        };
        assert_eq!(durations(&rig), [LibraryRequest::SetDuration(t(1), 210)]);

        rig.ended(first);
        rig.engine(EngineEvent::TrackStarted {
            token: second,
            duration_ms: Some(220),
        });
        assert_eq!(rig.view().current, Some(t(2)));
        assert_eq!(rig.view().duration_ms, 220);
        assert_eq!(durations(&rig)[1], LibraryRequest::SetDuration(t(2), 220));

        // The library knows this one.
        let third = rig.announced_token();
        rig.ended(second);
        rig.engine(EngineEvent::TrackStarted {
            token: third,
            duration_ms: Some(230),
        });
        assert_eq!(rig.view().current, Some(t(3)));
        assert_eq!(rig.view().duration_ms, 200);
        assert_eq!(durations(&rig).len(), 2, "{:?}", rig.requests);
    }

    #[test]
    fn no_device_pauses_with_a_notice_and_play_tries_again() {
        let music = Music::new().tone(300);
        let mut rig = Rig::with(&music);
        rig.factory.fail_open(1);

        rig.play(&[1], 0);
        rig.wait_until("the notice", |rig| {
            !rig.notices("playback-no-device").is_empty()
        });
        assert!(!rig.view().playing);
        assert_eq!(rig.view().current, Some(t(1)));

        rig.send(Command::TogglePlay);
        assert!(rig.view().playing);
        rig.run_until("the track", heard(rig.current_token()));
        assert_eq!(rig.notices.len(), 1, "{:?}", rig.notices);
    }

    #[test]
    fn a_lost_device_shows_a_notice_and_playback_goes_on() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1], 0);
        rig.wait_until("an output", |rig| rig.output().is_some());

        rig.output().unwrap().fail();
        rig.wait_until("the notice", |rig| {
            !rig.notices("playback-device-fallback").is_empty()
        });

        assert!(rig.view().playing);
        assert_eq!(rig.factory.outputs().len(), 2);
    }

    // Settings.

    #[test]
    fn settings_reach_the_engine_and_the_queue() {
        let music = Music::new().tone(300).tone(300).tone(300);
        let mut rig = Rig::with(&music);
        let device = DeviceId("speakers".into());
        rig.settings(&Settings {
            output_device: Some(device.clone()),
            gapless: false,
            repeat: RepeatMode::All,
            ..Settings::default()
        });

        rig.play(&[1], 0);
        rig.wait_until("an output", |rig| rig.output().is_some());
        assert_eq!(rig.output().unwrap().device(), Some(device));
        assert_eq!(rig.announced_track(), Some(t(1)), "repeat all wraps");

        rig.settings(&Settings {
            shuffle: true,
            ..Settings::default()
        });
        assert_eq!(rig.announced_track(), None, "repeat is off again");
        assert!(rig.service.shuffle);
    }

    #[test]
    fn unchanged_settings_change_nothing() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::with(&music);
        let settings = Settings {
            output_device: Some(DeviceId("speakers".into())),
            ..Settings::default()
        };
        rig.settings(&settings);
        rig.play(&[1], 0);
        rig.wait_until("an output", |rig| rig.output().is_some());
        rig.views.clear();

        // A second SetDevice would rebuild the output.
        rig.settings(&settings);
        rig.send(Command::TogglePlay);
        rig.send(Command::TogglePlay);
        rig.wait_until("the engine to catch up", |rig| rig.reading().playing);

        assert_eq!(rig.factory.outputs().len(), 1);
        assert_eq!(rig.views.len(), 2, "the two toggles and nothing else");
    }

    #[test]
    fn settings_are_applied_before_a_restore() {
        let music = Music::new().tone(3_000).tone(3_000);
        let device = DeviceId("speakers".into());
        let mut rig = Rig::idle();
        rig.settings(&Settings {
            output_device: Some(device.clone()),
            muted: true,
            repeat: RepeatMode::One,
            ..Settings::default()
        });
        rig.snapshot(music.snapshot());

        rig.restore(Some(record(&[1, 2])));

        assert_eq!(rig.announced_track(), Some(t(1)), "repeat one applies");
        rig.send(Command::TogglePlay);
        rig.run_until("the track to be heard", |rig| {
            rig.reading().position_ms > 100
        });
        assert_eq!(rig.output().unwrap().device(), Some(device));
        assert!(rig.pull().iter().all(|sample| *sample == 0.0));
    }

    // Recording plays.

    #[test]
    fn the_threshold_is_thirty_seconds() {
        assert_eq!(PLAY_COUNTS_AFTER_MS, 30_000);
        let rig = Rig::with(&Music::new());
        assert_eq!(rig.service.play_counts_after_ms, PLAY_COUNTS_AFTER_MS);
        assert!(
            rig.service.history,
            "history is on until the settings say otherwise"
        );
    }

    #[test]
    fn a_play_counts_after_thirty_seconds() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::with(&music).counting_soon();
        rig.play(&[1], 0);

        rig.run_until("most of the threshold", |rig| rig.position() >= 600);
        rig.tick();
        assert!(rig.position() < TEST_PLAY_MS, "{}", rig.position());
        assert!(plays(&rig).is_empty(), "{:?}", rig.requests);

        rig.run_until("the threshold", |rig| rig.position() >= TEST_PLAY_MS);
        assert!(plays(&rig).is_empty(), "a tick records it");
        rig.tick();
        assert_eq!(plays(&rig), [t(1)]);

        rig.run_until("twice the threshold", |rig| {
            rig.position() >= 2 * TEST_PLAY_MS
        });
        rig.tick();
        assert_eq!(plays(&rig), [t(1)], "a play is recorded once");
    }

    #[test]
    fn a_short_track_counts_when_it_ends() {
        let music = Music::new().tone(333).tone(3_000);
        let mut rig = Rig::with(&music).counting_soon();
        rig.play(&[1, 2], 0);

        // No tick at all: the track ends between two of them.
        rig.run_until("the track after it", |rig| {
            rig.service.queue.current() == Some(t(2))
        });

        assert_eq!(plays(&rig), [t(1)]);
    }

    #[test]
    fn a_skip_before_thirty_seconds_does_not_count() {
        let music = Music::new().tone(3_000).tone(3_000);
        let mut rig = Rig::with(&music).counting_soon();
        rig.play(&[1, 2], 0);
        rig.run_until("most of the threshold", |rig| rig.position() >= 600);
        rig.tick();
        assert!(rig.position() < TEST_PLAY_MS, "{}", rig.position());

        rig.send(Command::Next);
        rig.run_ticking_until("a play", |rig| !plays(rig).is_empty());

        assert_eq!(plays(&rig), [t(2)], "only the next track counts");
        assert!(
            rig.position() >= TEST_PLAY_MS,
            "the next track starts from nothing: {}",
            rig.position()
        );
    }

    #[test]
    fn a_pause_adds_nothing() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::with(&music).counting_soon();
        rig.play(&[1], 0);
        rig.run_until("some play", |rig| rig.position() >= 333);
        rig.send(Command::TogglePlay);
        let token = rig.current_token();
        rig.wait_until("the pause", |rig| {
            rig.reading().token == token && !rig.reading().playing
        });
        let paused_at = rig.position();

        // Twice the threshold in pulls, all of them paused.
        for _ in 0..44 {
            rig.pull();
            rig.pump();
            rig.tick();
        }

        // The pause fades out over a few milliseconds of audio, and then nothing moves.
        assert!(
            rig.position() < paused_at + 50,
            "{paused_at} -> {}",
            rig.position()
        );
        assert!(plays(&rig).is_empty(), "{:?}", rig.requests);
    }

    #[test]
    fn a_seek_adds_nothing_by_itself() {
        let music = Music::new().tone(10_000);
        let mut rig = Rig::with(&music).counting_soon();
        rig.play(&[1], 0);
        rig.run_until("some play", |rig| rig.position() >= 167);

        rig.send(Command::Seek(6_667));
        // While playing, the clock shows the seek once the device takes audio from there.
        rig.run_until("the seek", |rig| rig.position() >= 6_667);
        rig.tick();
        assert!(plays(&rig).is_empty(), "{:?}", rig.requests);

        rig.run_until("more play", |rig| rig.position() >= 6_667 + 333);
        rig.tick();
        assert!(plays(&rig).is_empty(), "{:?}", rig.requests);

        rig.run_ticking_until("a play", |rig| !plays(rig).is_empty());
        assert_eq!(plays(&rig), [t(1)]);
    }

    #[test]
    fn time_before_a_seek_still_counts() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::with(&music).counting_soon();
        rig.play(&[1], 0);
        rig.run_until("most of the threshold", |rig| rig.position() >= 667);
        let before = rig.position();

        rig.send(Command::Seek(0));
        rig.run_until("the seek", |rig| rig.position() < before);
        rig.tick();
        assert!(plays(&rig).is_empty(), "{:?}", rig.requests);

        rig.run_ticking_until("a play", |rig| !plays(rig).is_empty());
        assert_eq!(plays(&rig), [t(1)]);
        assert!(
            rig.position() < 700,
            "the time before the seek counts: {}",
            rig.position()
        );
    }

    #[test]
    fn repeat_one_counts_each_time_round() {
        let music = Music::new().tone(1_500);
        let mut rig = Rig::with(&music).counting_soon();
        rig.settings(&Settings {
            repeat: RepeatMode::One,
            ..Settings::default()
        });
        rig.play(&[1], 0);
        let rounds = |rig: &Rig| {
            let mut tokens: Vec<u64> = rig
                .views
                .iter()
                .filter(|view| view.current == Some(t(1)) && view.token != 0)
                .map(|view| view.token)
                .collect();
            tokens.dedup();
            tokens.len()
        };

        rig.run_ticking_until("the fourth time round", |rig| rounds(rig) >= 4);

        assert_eq!(plays(&rig), [t(1), t(1), t(1)]);
    }

    #[test]
    fn nothing_is_recorded_with_history_off() {
        let music = Music::new().tone(333).tone(3_000);
        let mut rig = Rig::with(&music).counting_soon();
        rig.settings(&Settings {
            history_enabled: false,
            ..Settings::default()
        });
        rig.play(&[1, 2], 0);

        rig.run_ticking_until("a full play", |rig| {
            rig.service.queue.current() == Some(t(2)) && rig.position() >= TEST_PLAY_MS
        });

        assert!(plays(&rig).is_empty(), "{:?}", rig.requests);
    }

    #[test]
    fn switching_history_off_mid_track_stops_the_record() {
        let music = Music::new().tone(1_500).tone(3_000);
        let mut rig = Rig::with(&music).counting_soon();
        rig.play(&[1, 2], 0);
        rig.run_until("some play", |rig| rig.position() >= 333);

        rig.settings(&Settings {
            history_enabled: false,
            ..Settings::default()
        });
        rig.run_ticking_until("the track after it", |rig| {
            rig.service.queue.current() == Some(t(2))
        });

        assert!(plays(&rig).is_empty(), "{:?}", rig.requests);
    }

    // Saving and restoring.

    #[test]
    fn plain_play_saves_the_lists_only_once() {
        let music = Music::new().tone(10_000).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2], 0);
        let start = Instant::now();
        rig.service.tick_at(start);
        rig.collect();
        assert_eq!(rig.saved_records().len(), 1, "the new context is saved");

        for n in 1..=5 {
            let before = rig.reading().position_ms;
            rig.run_until("more play", |rig| rig.reading().position_ms > before);
            rig.service.tick_at(start + SAVE_INTERVAL * n);
            rig.collect();
        }

        assert_eq!(rig.saved_records().len(), 1, "{:?}", rig.requests);
        assert_eq!(rig.view().current, Some(t(1)), "still the first track");
        let positions = rig.saved_positions();
        assert_eq!(positions.len(), 5, "{:?}", rig.requests);
        assert!(positions.iter().all(|(current, _)| *current == Some(t(1))));
        assert!(positions.windows(2).all(|pair| pair[0].1 < pair[1].1));

        rig.send(Command::AddToQueue(t(2)));
        rig.service.tick_at(start + SAVE_INTERVAL * 6);
        rig.collect();
        assert_eq!(
            rig.saved_records().len(),
            2,
            "a change to the lists is saved"
        );
    }

    #[test]
    fn the_queue_is_saved_every_interval_while_it_changes() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::with(&music);
        let start = Instant::now();
        rig.service.tick_at(start);
        rig.collect();
        assert!(rig.saved_records().is_empty(), "nothing has changed");

        rig.send(Command::AddToQueue(t(2)));
        rig.service.tick_at(start + Duration::from_secs(1));
        rig.collect();
        assert_eq!(rig.saved_records().len(), 1);

        rig.send(Command::AddToQueue(t(1)));
        rig.service.tick_at(start + SAVE_INTERVAL);
        rig.collect();
        assert_eq!(rig.saved_records().len(), 1, "too soon after the last");

        rig.service
            .tick_at(start + Duration::from_secs(1) + SAVE_INTERVAL);
        rig.collect();
        assert_eq!(rig.saved_records().len(), 2);

        rig.service
            .tick_at(start + Duration::from_secs(1) + SAVE_INTERVAL * 3);
        rig.collect();
        assert_eq!(rig.saved_records().len(), 2, "nothing has changed");
    }

    #[test]
    fn a_paused_queue_that_did_not_change_is_not_saved() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1], 0);
        rig.send(Command::TogglePlay);
        let token = rig.current_token();
        rig.wait_until("the pause", |rig| {
            rig.reading().token == token && !rig.reading().playing
        });
        let start = Instant::now();
        rig.service.tick_at(start);
        rig.service.tick_at(start + SAVE_INTERVAL);
        rig.service.finish();
        rig.collect();

        assert_eq!(rig.requests.len(), 1, "{:?}", rig.requests);
        assert_eq!(rig.saved_records().len(), 1);
    }

    #[test]
    fn exit_saves_a_position_that_moved_since_the_last_save() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1], 0);
        rig.service.tick_at(Instant::now());
        rig.collect();
        assert_eq!(rig.saved_records().len(), 1);

        rig.run_until("some play", |rig| rig.reading().position_ms > 100);
        rig.service.finish();
        rig.collect();

        assert_eq!(rig.saved_records().len(), 1, "the lists did not change");
        let positions = rig.saved_positions();
        assert_eq!(positions.len(), 1, "{:?}", rig.requests);
        assert!(positions[0].1 > 100);
    }

    #[test]
    fn nothing_is_saved_before_the_restore() {
        let music = Music::new().tone(300);
        let mut rig = Rig::idle();

        rig.service.tick_at(Instant::now());
        rig.service.finish();
        rig.collect();
        assert!(rig.requests.is_empty());

        rig.snapshot(music.snapshot());
        rig.service.tick_at(Instant::now());
        rig.service.finish();
        rig.collect();
        assert_eq!(rig.requests, [LibraryRequest::LoadQueue]);
    }

    #[test]
    fn exit_saves_the_queue_and_stops_the_sound() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1], 0);
        rig.run_until("some play", |rig| rig.reading().position_ms > 100);

        rig.service.finish();
        rig.collect();

        let saved = rig.saved_records()[0];
        assert_eq!(saved.current, Some(t(1)));
        assert!(saved.position_ms > 100);
        rig.wait_until("the stop", |rig| !rig.reading().playing);
        assert!(rig.service.into_engine().shutdown(DEADLINE));
    }

    #[test]
    fn a_position_at_the_end_restores_at_the_start() {
        let music = Music::new().tone(1_000).tone(1_000);
        let mut rig = Rig::idle();
        rig.snapshot(music.snapshot());

        rig.restore(Some(QueueRecord {
            position_ms: 1_000,
            ..record(&[1, 2])
        }));

        let token = rig.current_token();
        rig.wait_until("the load", |rig| rig.reading().token == token);
        assert_eq!(rig.reading().position_ms, 0);
        rig.pump();
        assert_eq!(rig.view().current, Some(t(1)), "it did not end at once");
        assert_eq!(rig.service.played_from, 0);
    }

    #[test]
    fn a_position_past_an_unknown_end_stays_paused() {
        let music = Music::new().untimed_tone(500).tone(500);
        let mut rig = Rig::idle();
        rig.snapshot(music.snapshot());

        rig.restore(Some(QueueRecord {
            position_ms: 60_000,
            ..record(&[1, 2])
        }));

        // The engine ends the track at once, and the next one waits for play.
        rig.wait_until("the next track", shows(2));
        assert!(!rig.view().playing);
        assert!(rig.factory.outputs().is_empty());
    }

    #[test]
    fn a_restored_position_is_not_a_second_of_play() {
        let music = Music::new().tone(3_000);
        let mut rig = Rig::idle();
        rig.snapshot(music.snapshot());
        rig.restore(Some(QueueRecord {
            position_ms: 2_000,
            ..record(&[1])
        }));
        let token = rig.current_token();
        rig.wait_until("the load", |rig| rig.reading().token == token);
        rig.service.failures = 3;

        rig.service.tick();

        assert!(rig.reading().position_ms >= FAILURE_RESET_MS);
        assert_eq!(rig.service.failures, 3);
    }

    #[test]
    fn a_restore_with_no_current_track_waits_for_play() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::idle();
        rig.snapshot(music.snapshot());

        rig.restore(Some(record(&[99, 2])));

        assert!(rig.service.current.is_none());
        assert_eq!(rig.view().current, None);
        assert_eq!(rig.upcoming(), [t(2)]);
        rig.send(Command::TogglePlay);
        assert_eq!(rig.view().current, Some(t(2)));
        assert!(rig.view().playing);
    }

    #[test]
    fn a_pressed_play_wins_over_the_restore() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::idle();
        rig.snapshot(music.snapshot());

        rig.play(&[2], 0);
        rig.restore(Some(record(&[1])));

        assert_eq!(rig.view().current, Some(t(2)));
        assert!(rig.view().playing);
        assert_eq!(rig.current_track(), t(2));
    }

    #[test]
    fn play_pressed_with_nothing_loaded_plays_what_the_restore_brings() {
        let music = Music::new().tone(300);
        let mut rig = Rig::idle();
        rig.send(Command::TogglePlay);
        rig.snapshot(music.snapshot());
        rig.send(Command::TogglePlay);
        rig.send(Command::TogglePlay);

        rig.restore(Some(record(&[1])));

        assert_eq!(rig.view().current, Some(t(1)));
        assert!(rig.view().playing, "three presses leave it on");
    }

    #[test]
    fn additions_before_the_restore_survive_it() {
        let music = Music::new().tone(300).tone(300);
        let mut rig = Rig::idle();
        rig.snapshot(music.snapshot());
        rig.send(Command::AddToQueue(t(2)));

        rig.restore(Some(QueueRecord {
            user: vec![t(1)],
            ..record(&[1])
        }));

        assert_eq!(rig.upcoming(), [t(1), t(2)]);
        assert!(!rig.view().playing);
        // Nothing else will change the lists, so the next save must carry them.
        rig.service.tick_at(Instant::now());
        rig.collect();
        let saved = rig.saved_records();
        assert_eq!(saved.len(), 1, "{:?}", rig.requests);
        assert_eq!(saved[0].user, [t(1), t(2)]);
    }

    #[test]
    fn a_dead_engine_never_shows_playing() {
        let music = Music::new().tone(3_000).tone(3_000);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2], 0);

        rig.service.engine_lost();
        rig.collect();
        assert!(!rig.view().playing);

        rig.send(Command::TogglePlay);
        assert!(!rig.view().playing);
        rig.send(Command::Next);
        assert!(!rig.view().playing);
        rig.play(&[2], 0);
        assert!(!rig.view().playing);
        assert_eq!(rig.notices("playback-no-device").len(), 2);
    }

    #[test]
    fn a_lost_library_ends_the_wait_for_the_queue() {
        let music = Music::new().tone(300);
        let mut rig = Rig::idle();
        rig.snapshot(music.snapshot());

        // The thread died with `LoadQueue` in flight, so no answer ever comes.
        rig.service.library_lost();
        rig.collect();

        assert_eq!(rig.service.restore, Restore::Done);
        // A late answer changes nothing either.
        rig.restore(Some(record(&[1])));
        assert!(rig.service.current.is_none());
    }

    #[test]
    fn a_lost_library_as_the_first_snapshot_asks_for_nothing() {
        let mut rig = Rig::idle();

        rig.service.library_lost();
        rig.collect();

        assert!(rig.requests.is_empty(), "{:?}", rig.requests);
        assert_eq!(rig.service.restore, Restore::Done);
        rig.send(Command::TogglePlay);
        assert!(rig.service.current.is_none());
    }

    #[test]
    fn a_lost_library_leaves_the_playing_track_alone() {
        let music = Music::new().tone(1_000).tone(300);
        let mut rig = Rig::with(&music);
        rig.play(&[1, 2], 0);
        let token = rig.current_token();

        rig.service.library_lost();
        rig.collect();

        assert_eq!(rig.current_token(), token);
        assert!(rig.view().playing);
        assert!(rig.upcoming().is_empty());
        rig.run_until("the stop", stopped);
    }
}
