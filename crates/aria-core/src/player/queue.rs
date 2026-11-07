//! The play queue: a pure model with no I/O and no threads.
//!
//! Two lists feed playback. The *user queue* ("Play next", "Add to queue") always plays before
//! the *context*, the list a track was started from. A context may hold the same `TrackId` more
//! than once, so every position here is an index, never a search by id.
//!
//! The context is walked through `order`, a permutation of context indexes: the identity with
//! shuffle off, a Fisher-Yates shuffle with shuffle on. `cursor` counts how many entries of
//! `order` have been consumed, so the next context track is `order[cursor]`. A user-queue track
//! playing does not move the cursor, which is how the context resumes where it left off.
//!
//! `peek_after_current` and `advance_finished` share one planning function, so the gapless
//! pre-load always names the track natural advancing will play.

use std::collections::VecDeque;

use aria_library::{QueueRecord, TrackId};
use serde::{Deserialize, Serialize};

/// Past this many milliseconds, Previous restarts the current track.
pub const RESTART_THRESHOLD_MS: u64 = 3_000;
/// How many played tracks Previous can walk back through.
pub const HISTORY_LIMIT: usize = 100;

/// What happens when the current track ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    /// Stop after the last track.
    #[default]
    Off,
    /// Wrap to the start of the context.
    All,
    /// Repeat the current track when it ends on its own.
    One,
}

/// The answer to a Previous request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviousAction {
    /// Seek the current track back to its start.
    Restart,
    /// Play this earlier track.
    Play(TrackId),
    /// There is no current track.
    Nothing,
}

/// One row of the upcoming list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpNext {
    pub track: TrackId,
    /// True for user-queue items, which can be dragged and removed.
    pub from_user: bool,
}

/// SplitMix64, a tiny seeded generator, so shuffles are reproducible from a seed.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n`; `n` must not be zero.
    fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }
}

/// The seed of the next pass, derived from the old one.
fn derive_seed(seed: u64) -> u64 {
    SplitMix64(seed).next_u64()
}

/// Fisher-Yates.
fn shuffle_in_place(items: &mut [usize], seed: u64) {
    let mut rng = SplitMix64(seed);
    for i in (1..items.len()).rev() {
        items.swap(i, rng.below(i + 1));
    }
}

/// A whole pass over `len` entries.
fn full_pass(len: usize, shuffled: bool, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    if shuffled {
        shuffle_in_place(&mut order, seed);
    }
    order
}

/// A played track that Previous can return to.
#[derive(Debug, Clone)]
struct Entry {
    track: TrackId,
    /// Its context index, when it played from the context.
    ctx: Option<usize>,
    /// The pass it played in; a context index only means something in that pass.
    pass: u64,
}

/// What advancing would do.
enum Plan {
    /// Repeat one: the same track again.
    Stay,
    /// Take the front of the user queue.
    User,
    /// Take this context index.
    Context(usize),
    /// Start a new pass of the context.
    Wrap { seed: u64, order: Vec<usize> },
}

/// The play queue.
#[derive(Debug, Clone)]
pub struct Queue {
    context: Vec<TrackId>,
    /// Playback order of context indexes. The identity unless shuffle is on.
    order: Vec<usize>,
    /// How many entries of `order` are consumed.
    cursor: usize,
    user: Vec<TrackId>,
    current: Option<TrackId>,
    /// The context index of `current`, when it is a context track. It is always
    /// `order[cursor - 1]` then.
    current_ctx: Option<usize>,
    history: VecDeque<Entry>,
    seed: u64,
    shuffle: bool,
    repeat: RepeatMode,
    /// Bumped whenever context indexes change meaning (new context, new pass).
    pass: u64,
}

impl Queue {
    pub fn new(seed: u64) -> Self {
        Self {
            context: Vec::new(),
            order: Vec::new(),
            cursor: 0,
            user: Vec::new(),
            current: None,
            current_ctx: None,
            history: VecDeque::new(),
            seed,
            shuffle: false,
            repeat: RepeatMode::Off,
            pass: 0,
        }
    }

    /// Makes `ids` the context and starts the entry at `start`. An empty list or an out of range
    /// `start` returns `None` and changes nothing. The user queue is kept.
    pub fn play_context(&mut self, ids: Vec<TrackId>, start: usize) -> Option<TrackId> {
        let track = ids.get(start).copied()?;
        self.push_history();
        let len = ids.len();
        self.pass = self.pass.wrapping_add(1);
        if self.shuffle {
            self.seed = derive_seed(self.seed);
            let mut rest: Vec<usize> = (0..len).filter(|&i| i != start).collect();
            shuffle_in_place(&mut rest, self.seed);
            self.order = std::iter::once(start).chain(rest).collect();
            self.cursor = 1;
        } else {
            self.order = (0..len).collect();
            self.cursor = start.saturating_add(1);
        }
        self.context = ids;
        self.current = Some(track);
        self.current_ctx = Some(start);
        Some(track)
    }

    /// Inserts at the front of the user queue.
    pub fn play_next(&mut self, id: TrackId) {
        self.user.insert(0, id);
    }

    /// Appends to the user queue.
    pub fn add_to_queue(&mut self, id: TrackId) {
        self.user.push(id);
    }

    /// Removes a user-queue item; an out of range index is ignored.
    pub fn remove_user(&mut self, index: usize) {
        if index < self.user.len() {
            self.user.remove(index);
        }
    }

    /// Moves a user-queue item so it ends up at `to`; out of range indexes are ignored.
    pub fn move_user(&mut self, from: usize, to: usize) {
        if from < self.user.len() && to < self.user.len() {
            let item = self.user.remove(from);
            self.user.insert(to, item);
        }
    }

    /// Empties the user queue. Context items are view-only and stay.
    pub fn clear_user(&mut self) {
        self.user.clear();
    }

    pub fn current(&self) -> Option<TrackId> {
        self.current
    }

    /// What `advance_finished` would return right now, without changing anything.
    pub fn peek_after_current(&self) -> Option<TrackId> {
        let plan = self.plan(false)?;
        self.plan_track(&plan)
    }

    /// Next pressed. Ignores repeat one and honors repeat all.
    pub fn advance_manual(&mut self) -> Option<TrackId> {
        self.advance(true)
    }

    /// The current track ended on its own.
    pub fn advance_finished(&mut self) -> Option<TrackId> {
        self.advance(false)
    }

    /// Previous pressed with the current track at `position_ms`.
    pub fn previous(&mut self, position_ms: u64) -> PreviousAction {
        let Some(current) = self.current else {
            return PreviousAction::Nothing;
        };
        if position_ms > RESTART_THRESHOLD_MS {
            return PreviousAction::Restart;
        }
        let Some(entry) = self.history.pop_back() else {
            return PreviousAction::Restart;
        };
        // Leave the current track so that going forward again plays it: a context track is
        // unconsumed, a user track goes back to the front of the user queue.
        match self.current_ctx {
            Some(c) => {
                if let Some(p) = self.pos_of(c) {
                    self.cursor = p;
                }
            }
            None => self.user.insert(0, current),
        }
        self.current = Some(entry.track);
        self.current_ctx = None;
        if entry.pass == self.pass
            && let Some(c) = entry.ctx
            && let Some(p) = self.pos_of(c)
        {
            self.cursor = p + 1;
            self.current_ctx = Some(c);
        }
        PreviousAction::Play(entry.track)
    }

    /// Turning shuffle on keeps the current track and shuffles only what is left of the pass.
    /// Turning it off restores the original order and continues after the current track.
    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        if on {
            self.seed = derive_seed(self.seed);
            let consumed = self.cursor.min(self.order.len());
            let mut taken = vec![false; self.context.len()];
            for &i in self.order.iter().take(consumed) {
                if let Some(flag) = taken.get_mut(i) {
                    *flag = true;
                }
            }
            let mut rest: Vec<usize> = (0..self.context.len())
                .filter(|&i| !taken.get(i).copied().unwrap_or(false))
                .collect();
            shuffle_in_place(&mut rest, self.seed);
            self.order.truncate(consumed);
            self.order.extend(rest);
        } else {
            let last = self
                .cursor
                .checked_sub(1)
                .and_then(|p| self.order.get(p))
                .copied();
            self.order = (0..self.context.len()).collect();
            self.cursor = last.map_or(0, |i| i + 1);
        }
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
    }

    /// Up to `limit` upcoming tracks: the user queue, then the rest of the current pass of the
    /// context. It does not look past the end of the pass, even with repeat all.
    pub fn upcoming(&self, limit: usize) -> Vec<UpNext> {
        let user = self.user.iter().map(|&track| UpNext {
            track,
            from_user: true,
        });
        let context = self
            .order
            .iter()
            .skip(self.cursor)
            .filter_map(|&i| self.context.get(i).copied())
            .map(|track| UpNext {
                track,
                from_user: false,
            });
        user.chain(context).take(limit).collect()
    }

    /// Drops tracks that no longer exist from the context, user queue and history. A vanished
    /// current track stays current until the caller moves on. Cursor and history stay valid, and
    /// the next surviving track stays next.
    pub fn retain(&mut self, exists: impl Fn(TrackId) -> bool) {
        // Old context index -> new context index.
        let mut map: Vec<Option<usize>> = Vec::with_capacity(self.context.len());
        let mut kept = Vec::new();
        for &track in &self.context {
            if exists(track) {
                map.push(Some(kept.len()));
                kept.push(track);
            } else {
                map.push(None);
            }
        }
        let remap = |i: usize| map.get(i).copied().flatten();
        let mut order = Vec::with_capacity(kept.len());
        let mut cursor = 0;
        for (p, &i) in self.order.iter().enumerate() {
            if let Some(n) = remap(i) {
                order.push(n);
                if p < self.cursor {
                    cursor = order.len();
                }
            }
        }
        self.current_ctx = self.current_ctx.and_then(remap);
        self.context = kept;
        self.order = order;
        self.cursor = cursor;
        self.user.retain(|&t| exists(t));
        self.history.retain_mut(|e| {
            if !exists(e.track) {
                return false;
            }
            e.ctx = e.ctx.and_then(remap);
            true
        });
    }

    /// The queue as saved to the database.
    ///
    /// `context` is stored in *playback order*, not the original order: with shuffle off they are
    /// the same, and with shuffle on it keeps the rest of the current pass exact without storing a
    /// permutation. `context_index` is the position of the last context track played (the current
    /// track when it came from the context), so everything after it is still to come.
    /// `shuffle_seed` seeds the next pass. The costs: a queue restored with shuffle on can no
    /// longer return to the pre-shuffle order, and its next pass (repeat all) is a fresh shuffle
    /// of the saved order rather than the one the unsaved queue would have drawn. History is not
    /// saved.
    pub fn to_record(&self, position_ms: u64) -> QueueRecord {
        QueueRecord {
            context: self
                .order
                .iter()
                .filter_map(|&i| self.context.get(i).copied())
                .collect(),
            user: self.user.clone(),
            current: self.current,
            position_ms,
            shuffle_seed: self.seed,
            context_index: self.cursor.checked_sub(1),
        }
    }

    /// Rebuilds a queue from a saved record, dropping tracks `exists` rejects. Without its
    /// current track, `current` is `None` and playback continues with the following context
    /// track. A `context_index` beyond the context counts as the whole context played.
    pub fn from_record(
        record: QueueRecord,
        shuffle: bool,
        repeat: RepeatMode,
        exists: impl Fn(TrackId) -> bool,
    ) -> Self {
        let QueueRecord {
            context,
            user,
            current,
            shuffle_seed,
            context_index,
            ..
        } = record;
        let consumed = context_index.map_or(0, |i| i.saturating_add(1).min(context.len()));
        let current_old =
            context_index.filter(|&i| current.is_some() && context.get(i).copied() == current);
        let mut kept = Vec::with_capacity(context.len());
        let mut cursor = 0;
        let mut current_ctx = None;
        for (i, &track) in context.iter().enumerate() {
            if !exists(track) {
                continue;
            }
            if current_old == Some(i) {
                current_ctx = Some(kept.len());
            }
            kept.push(track);
            if i < consumed {
                cursor = kept.len();
            }
        }
        let current = current.filter(|&t| exists(t));
        if current.is_none() {
            current_ctx = None;
        }
        Self {
            order: (0..kept.len()).collect(),
            context: kept,
            cursor,
            user: user.into_iter().filter(|&t| exists(t)).collect(),
            current,
            current_ctx,
            history: VecDeque::new(),
            seed: shuffle_seed,
            shuffle,
            repeat,
            pass: 0,
        }
    }

    fn pos_of(&self, ctx_index: usize) -> Option<usize> {
        self.order.iter().position(|&i| i == ctx_index)
    }

    fn push_history(&mut self) {
        if let Some(track) = self.current {
            self.history.push_back(Entry {
                track,
                ctx: self.current_ctx,
                pass: self.pass,
            });
            while self.history.len() > HISTORY_LIMIT {
                self.history.pop_front();
            }
        }
    }

    fn plan(&self, manual: bool) -> Option<Plan> {
        if !manual && self.repeat == RepeatMode::One && self.current.is_some() {
            return Some(Plan::Stay);
        }
        if !self.user.is_empty() {
            return Some(Plan::User);
        }
        if let Some(&i) = self.order.get(self.cursor) {
            return Some(Plan::Context(i));
        }
        if self.repeat == RepeatMode::All && !self.context.is_empty() {
            let seed = if self.shuffle {
                derive_seed(self.seed)
            } else {
                self.seed
            };
            return Some(Plan::Wrap {
                seed,
                order: full_pass(self.context.len(), self.shuffle, seed),
            });
        }
        None
    }

    fn plan_track(&self, plan: &Plan) -> Option<TrackId> {
        match plan {
            Plan::Stay => self.current,
            Plan::User => self.user.first().copied(),
            Plan::Context(i) => self.context.get(*i).copied(),
            Plan::Wrap { order, .. } => order.first().and_then(|&i| self.context.get(i).copied()),
        }
    }

    fn advance(&mut self, manual: bool) -> Option<TrackId> {
        let plan = self.plan(manual)?;
        let track = self.plan_track(&plan)?;
        match plan {
            Plan::Stay => {}
            Plan::User => {
                self.push_history();
                self.user.remove(0);
                self.current = Some(track);
                self.current_ctx = None;
            }
            Plan::Context(i) => {
                self.push_history();
                self.cursor += 1;
                self.current = Some(track);
                self.current_ctx = Some(i);
            }
            Plan::Wrap { seed, order } => {
                self.push_history();
                self.current_ctx = order.first().copied();
                self.seed = seed;
                self.order = order;
                self.cursor = 1;
                self.pass = self.pass.wrapping_add(1);
                self.current = Some(track);
            }
        }
        Some(track)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: i64) -> Vec<TrackId> {
        (0..n).map(TrackId).collect()
    }

    fn t(n: i64) -> TrackId {
        TrackId(n)
    }

    fn user_items(q: &Queue) -> Vec<TrackId> {
        q.upcoming(usize::MAX)
            .into_iter()
            .filter(|u| u.from_user)
            .map(|u| u.track)
            .collect()
    }

    fn context_items(q: &Queue) -> Vec<TrackId> {
        q.upcoming(usize::MAX)
            .into_iter()
            .filter(|u| !u.from_user)
            .map(|u| u.track)
            .collect()
    }

    /// Plays the whole current pass with `advance_finished`, starting with the current track.
    fn drain(q: &mut Queue) -> Vec<TrackId> {
        let mut out: Vec<TrackId> = q.current().into_iter().collect();
        while let Some(next) = q.advance_finished() {
            out.push(next);
        }
        out
    }

    fn check_invariants(q: &Queue, what: &str) {
        assert_eq!(q.order.len(), q.context.len(), "{what}: order length");
        let mut sorted = q.order.clone();
        sorted.sort_unstable();
        assert!(
            sorted.iter().copied().eq(0..q.context.len()),
            "{what}: order is not a permutation"
        );
        assert!(q.cursor <= q.order.len(), "{what}: cursor out of range");
        if !q.shuffle {
            assert!(
                q.order.iter().copied().eq(0..q.context.len()),
                "{what}: order not identity with shuffle off"
            );
        }
        if let Some(c) = q.current_ctx {
            assert!(q.cursor > 0, "{what}: current_ctx with cursor 0");
            assert_eq!(
                q.order.get(q.cursor - 1),
                Some(&c),
                "{what}: current_ctx is not the last consumed"
            );
            assert_eq!(
                q.context.get(c).copied(),
                q.current,
                "{what}: current_ctx does not hold the current track"
            );
        }
        assert!(q.history.len() <= HISTORY_LIMIT, "{what}: history bound");
    }

    #[test]
    fn context_plays_in_order_from_start() {
        let mut q = Queue::new(1);
        assert_eq!(q.play_context(ids(5), 2), Some(t(2)));
        assert_eq!(q.current(), Some(t(2)));
        assert_eq!(q.advance_manual(), Some(t(3)));
        assert_eq!(q.advance_manual(), Some(t(4)));
        assert_eq!(q.advance_manual(), None);
        assert_eq!(q.current(), Some(t(4)), "the last track stays loaded");
    }

    #[test]
    fn user_queue_plays_before_context_then_context_resumes() {
        let mut q = Queue::new(1);
        q.play_context(ids(4), 0);
        q.add_to_queue(t(10));
        q.add_to_queue(t(11));
        assert_eq!(q.advance_finished(), Some(t(10)));
        assert_eq!(q.advance_finished(), Some(t(11)));
        assert_eq!(q.advance_finished(), Some(t(1)));
        assert_eq!(q.advance_finished(), Some(t(2)));
    }

    #[test]
    fn play_next_goes_first_add_goes_last() {
        let mut q = Queue::new(1);
        q.play_context(ids(3), 0);
        q.add_to_queue(t(10));
        q.add_to_queue(t(11));
        q.play_next(t(12));
        assert_eq!(user_items(&q), vec![t(12), t(10), t(11)]);
        assert_eq!(q.peek_after_current(), Some(t(12)));
    }

    #[test]
    fn remove_and_move_user_items() {
        let mut q = Queue::new(1);
        for n in 1..=4 {
            q.add_to_queue(t(n));
        }
        q.remove_user(9);
        assert_eq!(user_items(&q), vec![t(1), t(2), t(3), t(4)]);
        q.remove_user(1);
        assert_eq!(user_items(&q), vec![t(1), t(3), t(4)]);
        q.move_user(0, 2);
        assert_eq!(user_items(&q), vec![t(3), t(4), t(1)]);
        q.move_user(5, 0);
        q.move_user(0, 5);
        assert_eq!(user_items(&q), vec![t(3), t(4), t(1)]);
        q.move_user(2, 0);
        assert_eq!(user_items(&q), vec![t(1), t(3), t(4)]);
    }

    #[test]
    fn clear_keeps_context() {
        let mut q = Queue::new(1);
        q.play_context(ids(4), 0);
        q.add_to_queue(t(10));
        q.play_next(t(11));
        q.clear_user();
        assert!(user_items(&q).is_empty());
        assert_eq!(context_items(&q), vec![t(1), t(2), t(3)]);
        assert_eq!(q.current(), Some(t(0)));
    }

    #[test]
    fn repeat_off_ends_at_last() {
        let mut q = Queue::new(1);
        q.play_context(ids(2), 0);
        assert_eq!(q.advance_finished(), Some(t(1)));
        assert_eq!(q.peek_after_current(), None);
        assert_eq!(q.advance_finished(), None);
        assert_eq!(q.advance_manual(), None);
        assert_eq!(q.current(), Some(t(1)));
    }

    #[test]
    fn repeat_all_wraps() {
        let mut q = Queue::new(1);
        q.set_repeat(RepeatMode::All);
        q.play_context(ids(3), 1);
        assert_eq!(q.advance_finished(), Some(t(2)));
        assert_eq!(q.peek_after_current(), Some(t(0)));
        assert_eq!(q.advance_finished(), Some(t(0)));
        assert_eq!(q.advance_manual(), Some(t(1)));
        assert_eq!(q.advance_manual(), Some(t(2)));
        assert_eq!(q.advance_manual(), Some(t(0)), "manual honors repeat all");
    }

    #[test]
    fn repeat_one_repeats_on_finish_but_next_advances() {
        let mut q = Queue::new(1);
        q.set_repeat(RepeatMode::One);
        q.play_context(ids(3), 0);
        assert_eq!(q.peek_after_current(), Some(t(0)));
        assert_eq!(q.advance_finished(), Some(t(0)));
        assert_eq!(q.advance_finished(), Some(t(0)));
        assert_eq!(q.advance_manual(), Some(t(1)));
        assert_eq!(q.advance_finished(), Some(t(1)));
        assert_eq!(q.advance_manual(), Some(t(2)));
        assert_eq!(q.advance_manual(), None, "repeat one does not wrap on next");
    }

    #[test]
    fn peek_matches_finished_advance() {
        for seed in 0..200u64 {
            let mut rng = SplitMix64(seed);
            let mut q = Queue::new(seed);
            for step in 0..60 {
                let what = format!("seed {seed} step {step}");
                let peeked = q.peek_after_current();
                let mut probe = q.clone();
                assert_eq!(peeked, probe.advance_finished(), "{what}: peek");
                check_invariants(&probe, &what);
                match rng.below(14) {
                    0 | 1 => {
                        let len = rng.below(8) as i64;
                        let list: Vec<TrackId> =
                            (0..len).map(|_| TrackId(rng.below(6) as i64)).collect();
                        let start = rng.below(9);
                        q.play_context(list, start);
                    }
                    2 => q.play_next(TrackId(100 + rng.below(4) as i64)),
                    3 => q.add_to_queue(TrackId(100 + rng.below(4) as i64)),
                    4 => q.remove_user(rng.below(4)),
                    5 => q.move_user(rng.below(4), rng.below(4)),
                    6 => q.clear_user(),
                    7 | 8 => {
                        q.advance_manual();
                    }
                    9 | 10 => {
                        q.advance_finished();
                    }
                    11 => {
                        let pos = if rng.below(2) == 0 { 0 } else { 5_000 };
                        q.previous(pos);
                    }
                    12 => match rng.below(3) {
                        0 => q.set_shuffle(rng.below(2) == 0),
                        1 => q.set_repeat(RepeatMode::All),
                        _ => q.set_repeat(if rng.below(2) == 0 {
                            RepeatMode::One
                        } else {
                            RepeatMode::Off
                        }),
                    },
                    _ => {
                        let gone = rng.below(7) as i64;
                        q.retain(|id| id.0 % 7 != gone);
                    }
                }
                check_invariants(&q, &what);
            }
        }
    }

    #[test]
    fn shuffle_visits_each_track_once_per_pass() {
        let mut q = Queue::new(7);
        q.set_shuffle(true);
        q.set_repeat(RepeatMode::All);
        q.play_context(ids(100), 0);
        assert_eq!(q.current(), Some(t(0)), "the started track plays first");
        let mut first = vec![t(0)];
        for _ in 1..100 {
            first.extend(q.advance_finished());
        }
        let mut sorted = first.clone();
        sorted.sort();
        assert_eq!(sorted, ids(100), "every track exactly once");
        assert_ne!(first, ids(100), "the order is shuffled");
        let second: Vec<TrackId> = (0..100).filter_map(|_| q.advance_finished()).collect();
        let mut sorted = second.clone();
        sorted.sort();
        assert_eq!(sorted, ids(100), "the next pass also visits each once");
        assert_ne!(second, first, "each pass reshuffles");
    }

    #[test]
    fn shuffle_with_duplicate_ids_visits_each_entry() {
        let mut q = Queue::new(3);
        q.set_shuffle(true);
        q.play_context(vec![t(1), t(2), t(1), t(1), t(3)], 2);
        let mut seen = drain(&mut q);
        seen.sort();
        assert_eq!(seen, vec![t(1), t(1), t(1), t(2), t(3)]);
    }

    #[test]
    fn duplicate_ids_in_context_play_by_position() {
        let mut q = Queue::new(1);
        q.play_context(vec![t(1), t(2), t(1), t(3)], 2);
        assert_eq!(drain(&mut q), vec![t(1), t(3)]);
        q.play_context(vec![t(1), t(2), t(1), t(3)], 0);
        assert_eq!(drain(&mut q), vec![t(1), t(2), t(1), t(3)]);
    }

    #[test]
    fn shuffle_on_keeps_current_and_shuffles_only_the_rest() {
        let mut q = Queue::new(5);
        q.play_context(ids(30), 10);
        assert_eq!(q.advance_manual(), Some(t(11)));
        q.set_shuffle(true);
        assert_eq!(q.current(), Some(t(11)));
        let rest = context_items(&q);
        let mut sorted = rest.clone();
        sorted.sort();
        assert_eq!(sorted, (12..30).map(t).collect::<Vec<_>>());
        assert_ne!(rest, (12..30).map(t).collect::<Vec<_>>());
    }

    #[test]
    fn shuffle_off_restores_order_and_keeps_current() {
        let mut q = Queue::new(9);
        q.set_shuffle(true);
        q.play_context(ids(10), 3);
        q.advance_manual();
        q.advance_manual();
        let current = q.current();
        q.set_shuffle(false);
        assert_eq!(q.current(), current);
        let k = current.map_or(0, |c| c.0);
        let expected: Vec<TrackId> = (k + 1..10).map(t).collect();
        assert_eq!(context_items(&q), expected);
        assert_eq!(drain(&mut q)[1..], expected[..]);
    }

    #[test]
    fn previous_restarts_after_three_seconds() {
        let mut q = Queue::new(1);
        q.play_context(ids(3), 0);
        q.advance_manual();
        assert_eq!(q.previous(3_001), PreviousAction::Restart);
        assert_eq!(q.current(), Some(t(1)));
        assert_eq!(q.previous(2_999), PreviousAction::Play(t(0)));
        assert_eq!(q.current(), Some(t(0)));
    }

    #[test]
    fn previous_edge_cases() {
        let mut q = Queue::new(1);
        assert_eq!(q.previous(0), PreviousAction::Nothing);
        assert_eq!(q.previous(10_000), PreviousAction::Nothing);
        q.play_context(ids(3), 1);
        assert_eq!(q.previous(0), PreviousAction::Restart, "empty history");
    }

    #[test]
    fn previous_walks_history_after_shuffle() {
        let mut q = Queue::new(11);
        q.set_shuffle(true);
        q.play_context(ids(20), 4);
        let mut played = vec![q.current()];
        for _ in 0..5 {
            played.push(q.advance_manual());
        }
        let mut back = Vec::new();
        for _ in 0..5 {
            match q.previous(0) {
                PreviousAction::Play(id) => back.push(Some(id)),
                other => panic!("expected a track, got {other:?}"),
            }
        }
        let mut expected: Vec<_> = played[..5].to_vec();
        expected.reverse();
        assert_eq!(back, expected);
        assert_eq!(q.previous(0), PreviousAction::Restart);
        // Going forward again replays the same sequence: nothing skipped or repeated.
        let forward: Vec<_> = (0..5).map(|_| q.advance_manual()).collect();
        assert_eq!(forward, played[1..].to_vec());
    }

    #[test]
    fn previous_restores_context_position() {
        let mut q = Queue::new(1);
        q.play_context(ids(6), 0);
        q.advance_manual();
        q.advance_manual();
        assert_eq!(q.current(), Some(t(2)));
        assert_eq!(q.previous(0), PreviousAction::Play(t(1)));
        assert_eq!(q.advance_manual(), Some(t(2)), "resumes, no skip");
        assert_eq!(q.advance_manual(), Some(t(3)));
        assert_eq!(q.previous(0), PreviousAction::Play(t(2)));
        assert_eq!(q.previous(0), PreviousAction::Play(t(1)));
        assert_eq!(q.previous(0), PreviousAction::Play(t(0)));
        assert_eq!(q.advance_manual(), Some(t(1)), "no repeat of the first");
    }

    #[test]
    fn user_track_does_not_move_context_and_previous_returns() {
        let mut q = Queue::new(1);
        q.play_context(ids(4), 0);
        q.advance_manual(); // 1
        q.add_to_queue(t(50));
        assert_eq!(q.advance_manual(), Some(t(50)));
        assert_eq!(q.previous(0), PreviousAction::Play(t(1)));
        assert_eq!(
            q.advance_manual(),
            Some(t(50)),
            "the user track is not lost"
        );
        assert_eq!(q.advance_manual(), Some(t(2)), "context continues unmoved");
        // Back from the context track into the user track and on.
        assert_eq!(q.previous(0), PreviousAction::Play(t(50)));
        assert_eq!(q.previous(0), PreviousAction::Play(t(1)));
        assert_eq!(q.advance_manual(), Some(t(50)));
        assert_eq!(q.advance_manual(), Some(t(2)));
        assert_eq!(q.advance_manual(), Some(t(3)));
    }

    #[test]
    fn previous_into_a_user_track_then_forward_resumes_the_context_track() {
        let mut q = Queue::new(1);
        q.play_context(ids(4), 0);
        q.advance_manual(); // 1
        q.add_to_queue(t(50));
        q.advance_manual(); // 50
        assert_eq!(q.advance_manual(), Some(t(2)));
        assert_eq!(q.previous(0), PreviousAction::Play(t(50)));
        check_invariants(&q, "in the user track");
        assert_eq!(
            q.advance_manual(),
            Some(t(2)),
            "the track left is not skipped"
        );
        assert_eq!(q.advance_manual(), Some(t(3)));
    }

    #[test]
    fn history_is_bounded() {
        let mut q = Queue::new(1);
        q.play_context(ids(300), 0);
        for _ in 0..250 {
            q.advance_manual();
        }
        let mut back = 0;
        while let PreviousAction::Play(_) = q.previous(0) {
            back += 1;
        }
        assert_eq!(back, HISTORY_LIMIT);
    }

    #[test]
    fn play_context_rejects_empty_and_out_of_range() {
        let mut q = Queue::new(1);
        q.play_context(ids(3), 1);
        q.add_to_queue(t(9));
        assert_eq!(q.play_context(Vec::new(), 0), None);
        assert_eq!(q.play_context(ids(3), 3), None);
        assert_eq!(q.current(), Some(t(1)));
        assert_eq!(user_items(&q), vec![t(9)]);
        assert_eq!(context_items(&q), vec![t(2)]);
        check_invariants(&q, "after rejected play_context");
    }

    fn build_busy_queue(shuffle: bool, repeat: RepeatMode) -> Queue {
        let mut q = Queue::new(21);
        q.set_repeat(repeat);
        q.set_shuffle(shuffle);
        q.play_context(ids(12), 3);
        q.advance_manual();
        q.advance_manual();
        q.add_to_queue(t(90));
        q.add_to_queue(t(91));
        q
    }

    fn assert_same_future(a: &mut Queue, b: &mut Queue, steps: usize) {
        for step in 0..steps {
            assert_eq!(a.upcoming(100), b.upcoming(100), "upcoming at {step}");
            assert_eq!(
                a.peek_after_current(),
                b.peek_after_current(),
                "peek {step}"
            );
            assert_eq!(a.advance_finished(), b.advance_finished(), "advance {step}");
        }
    }

    #[test]
    fn record_round_trip() {
        for shuffle in [false, true] {
            for repeat in [RepeatMode::Off, RepeatMode::All] {
                let mut original = build_busy_queue(shuffle, repeat);
                let record = original.to_record(1_234);
                assert_eq!(record.position_ms, 1_234);
                assert_eq!(record.current, original.current());
                assert_eq!(record.user, vec![t(90), t(91)]);
                let mut restored = Queue::from_record(record, shuffle, repeat, |_| true);
                assert_eq!(restored.current(), original.current());
                check_invariants(&restored, "restored");
                // The user queue and the rest of the pass come back identically.
                let remaining = original.upcoming(100).len();
                assert_same_future(&mut original, &mut restored, remaining);
                // The next pass is a fresh shuffle either way, and still covers every track.
                if repeat == RepeatMode::All {
                    let mut a: Vec<TrackId> = (0..12)
                        .filter_map(|_| original.advance_finished())
                        .collect();
                    let mut b: Vec<TrackId> = (0..12)
                        .filter_map(|_| restored.advance_finished())
                        .collect();
                    a.sort();
                    b.sort();
                    assert_eq!(a, ids(12));
                    assert_eq!(b, ids(12));
                } else {
                    assert_eq!(original.advance_finished(), None);
                    assert_eq!(restored.advance_finished(), None);
                }
            }
        }
    }

    #[test]
    fn record_round_trip_with_user_track_playing() {
        let mut original = build_busy_queue(true, RepeatMode::All);
        assert_eq!(original.advance_manual(), Some(t(90)));
        let record = original.to_record(0);
        let mut restored = Queue::from_record(record, true, RepeatMode::All, |_| true);
        assert_eq!(restored.current(), Some(t(90)));
        let remaining = original.upcoming(100).len();
        assert_same_future(&mut original, &mut restored, remaining);
    }

    #[test]
    fn restore_drops_missing_tracks() {
        let original = build_busy_queue(false, RepeatMode::Off);
        let record = original.to_record(0);
        let q = Queue::from_record(record, false, RepeatMode::Off, |id| id.0 != 6 && id.0 != 91);
        assert_eq!(context_items(&q), vec![t(7), t(8), t(9), t(10), t(11)]);
        assert_eq!(user_items(&q), vec![t(90)]);
        assert_eq!(q.current(), Some(t(5)));
        check_invariants(&q, "restore_drops_missing_tracks");
    }

    #[test]
    fn restore_missing_current_continues_with_following_track() {
        let original = build_busy_queue(false, RepeatMode::Off);
        assert_eq!(original.current(), Some(t(5)));
        let record = original.to_record(0);
        let mut q = Queue::from_record(record, false, RepeatMode::Off, |id| id.0 != 5);
        assert_eq!(q.current(), None);
        check_invariants(&q, "missing current");
        assert_eq!(q.advance_manual(), Some(t(90)));
        assert_eq!(q.advance_manual(), Some(t(91)));
        assert_eq!(q.advance_manual(), Some(t(6)), "following context track");
    }

    #[test]
    fn restore_with_out_of_range_context_index_is_safe() {
        for index in [0, 5, 99, usize::MAX] {
            let record = QueueRecord {
                context: ids(5),
                user: vec![t(8)],
                current: Some(t(2)),
                position_ms: 10,
                shuffle_seed: 4,
                context_index: Some(index),
            };
            let mut q = Queue::from_record(record, true, RepeatMode::All, |_| true);
            check_invariants(&q, "out of range index");
            for _ in 0..12 {
                q.advance_finished();
                q.previous(0);
            }
            check_invariants(&q, "out of range index after use");
        }
    }

    #[test]
    fn retain_removes_gone_tracks_and_keeps_current() {
        let mut q = Queue::new(1);
        q.play_context(ids(8), 2);
        q.add_to_queue(t(50));
        q.add_to_queue(t(51));
        q.advance_manual(); // 50
        q.advance_manual(); // 51
        q.advance_manual(); // 3
        q.advance_manual(); // 4
        q.add_to_queue(t(52));
        // Current track 4 vanishes, along with 5, user item 52 and history item 51.
        q.retain(|id| ![4, 5, 52, 51].contains(&id.0));
        check_invariants(&q, "after retain");
        assert_eq!(q.current(), Some(t(4)), "vanished current stays current");
        assert!(user_items(&q).is_empty());
        assert_eq!(context_items(&q), vec![t(6), t(7)]);
        assert_eq!(q.peek_after_current(), Some(t(6)));
        assert_eq!(q.advance_manual(), Some(t(6)));
        assert_eq!(q.previous(0), PreviousAction::Play(t(4)));
        assert_eq!(q.previous(0), PreviousAction::Play(t(3)));
        assert_eq!(q.previous(0), PreviousAction::Play(t(50)), "51 was dropped");
    }

    #[test]
    fn retain_keeps_next_track_while_it_exists() {
        let mut q = Queue::new(3);
        q.set_shuffle(true);
        q.play_context(ids(20), 0);
        for _ in 0..4 {
            q.advance_manual();
        }
        let next = q.peek_after_current();
        let current = q.current();
        q.retain(|id| Some(id) == next || Some(id) == current || id.0 % 2 == 0);
        check_invariants(&q, "retain keeps next");
        assert_eq!(q.peek_after_current(), next);
        q.retain(|_| false);
        check_invariants(&q, "retain all");
        assert_eq!(q.peek_after_current(), None);
        assert_eq!(q.current(), current);
    }

    #[test]
    fn empty_context_is_safe() {
        let mut q = Queue::new(0);
        assert_eq!(q.current(), None);
        assert_eq!(q.peek_after_current(), None);
        assert_eq!(q.advance_manual(), None);
        assert_eq!(q.advance_finished(), None);
        assert_eq!(q.previous(0), PreviousAction::Nothing);
        assert_eq!(q.previous(9_999), PreviousAction::Nothing);
        q.remove_user(0);
        q.move_user(0, 1);
        q.clear_user();
        q.set_shuffle(true);
        q.set_repeat(RepeatMode::All);
        assert_eq!(q.advance_manual(), None);
        q.set_repeat(RepeatMode::One);
        assert_eq!(q.peek_after_current(), None);
        q.set_repeat(RepeatMode::Off);
        q.set_shuffle(false);
        q.retain(|_| true);
        assert!(q.upcoming(5).is_empty());
        assert!(q.upcoming(0).is_empty());
        let record = q.to_record(5);
        assert!(record.context.is_empty() && record.current.is_none());
        let restored = Queue::from_record(record, true, RepeatMode::All, |_| true);
        assert_eq!(restored.current(), None);
        let blank = Queue::from_record(QueueRecord::default(), false, RepeatMode::Off, |_| false);
        assert_eq!(blank.peek_after_current(), None);
        // Only user items: they play, then it ends.
        q.add_to_queue(t(1));
        assert_eq!(q.advance_finished(), Some(t(1)));
        assert_eq!(q.advance_finished(), None);
    }

    #[test]
    fn repeat_mode_serializes_lowercase() {
        assert_eq!(RepeatMode::default(), RepeatMode::Off);
        let text = toml::to_string(&std::collections::BTreeMap::from([("r", RepeatMode::All)]))
            .unwrap_or_default();
        assert_eq!(text.trim(), "r = \"all\"");
    }
}
