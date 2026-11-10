//! Where playback is, for any thread to read without waiting.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// What is being heard at one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClockReading {
    /// The token of the track being heard, or 0 when there has been none.
    pub token: u64,
    pub position_ms: u64,
    pub playing: bool,
}

#[derive(Default)]
struct Slot {
    token: AtomicU64,
    position_ms: AtomicU64,
    playing: AtomicBool,
}

/// The latest [`ClockReading`], shared through an `Arc`.
///
/// A reading is three values, so two slots take turns: the writer fills the slot not in use, then
/// makes it the current one. A reader never waits for the writer, and tries again only when a whole
/// new reading arrived while it was looking. Everything is `SeqCst`, which makes that check a
/// matter of order alone, and costs nothing at a hundred readings a second.
#[derive(Default)]
pub struct PlaybackClock {
    slots: [Slot; 2],
    /// Counts readings. Its low bit names the current slot.
    version: AtomicU64,
}

impl PlaybackClock {
    /// The latest reading, with its values all from the same moment.
    pub fn read(&self) -> ClockReading {
        loop {
            let version = self.version.load(Ordering::SeqCst);
            let slot = &self.slots[(version % 2) as usize];
            let reading = ClockReading {
                token: slot.token.load(Ordering::SeqCst),
                position_ms: slot.position_ms.load(Ordering::SeqCst),
                playing: slot.playing.load(Ordering::SeqCst),
            };
            if self.version.load(Ordering::SeqCst) == version {
                return reading;
            }
        }
    }

    /// Replace the reading. Only one thread may do this: the engine, or a test standing in for it.
    pub fn publish(&self, reading: ClockReading) {
        let version = self.version.load(Ordering::SeqCst) + 1;
        let slot = &self.slots[(version % 2) as usize];
        slot.token.store(reading.token, Ordering::SeqCst);
        slot.position_ms
            .store(reading.position_ms, Ordering::SeqCst);
        slot.playing.store(reading.playing, Ordering::SeqCst);
        self.version.store(version, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_nothing() {
        assert_eq!(PlaybackClock::default().read(), ClockReading::default());
    }

    #[test]
    fn reads_back_what_was_published() {
        let clock = PlaybackClock::default();
        let reading = ClockReading {
            token: 7,
            position_ms: 1_234,
            playing: true,
        };

        clock.publish(reading);
        assert_eq!(clock.read(), reading);

        clock.publish(ClockReading::default());
        assert_eq!(clock.read(), ClockReading::default());
    }

    #[test]
    fn a_reading_never_mixes_two_moments() {
        const READINGS: u64 = 200_000;

        let clock = PlaybackClock::default();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for count in 1..=READINGS {
                    clock.publish(ClockReading {
                        token: count,
                        position_ms: count * 2,
                        playing: count % 2 == 0,
                    });
                }
            });
            loop {
                let reading = clock.read();
                assert_eq!(reading.position_ms, reading.token * 2);
                if reading.token > 0 {
                    assert_eq!(reading.playing, reading.token % 2 == 0);
                }
                if reading.token == READINGS {
                    break;
                }
            }
        });
    }
}
