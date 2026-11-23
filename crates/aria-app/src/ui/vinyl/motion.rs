//! How the record and the tonearm move: the record turns at the chosen speed while playing,
//! speeding up and slowing down smoothly, and the arm swings on and off the record.

use std::f32::consts::TAU;

use aria_core::settings::{VINYL_RPM_MAX, VINYL_RPM_MIN};

/// The marks on the speed slider, in turns per minute: 20 is Aria's own, the rest are the
/// speeds records are made for.
pub const RPM_MARKS: [f32; 5] = [10.0, 20.0, 100.0 / 3.0, 45.0, 78.0];
/// How close to a mark a chosen speed snaps to it.
const SNAP: f32 = 1.5;

/// `rpm` within the slider's range, on a mark when it is near one.
pub fn snap_rpm(rpm: f32) -> f32 {
    let rpm = rpm.clamp(VINYL_RPM_MIN, VINYL_RPM_MAX);
    RPM_MARKS
        .iter()
        .copied()
        .find(|mark| (rpm - mark).abs() <= SNAP)
        .unwrap_or(rpm)
}
/// From still to full speed, in seconds.
const SPIN_UP: f32 = 0.6;
/// From full speed to still, in seconds.
const SPIN_DOWN: f32 = 0.8;
/// The arm's swing either way, in seconds.
const ARM: f32 = 0.5;

/// The turntable's moving parts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Turntable {
    /// The record's angle, in radians, within `0..TAU`.
    pub angle: f32,
    /// The record's speed, from 0 (still) to 1 (full speed).
    pub speed: f32,
    /// The arm, from 0 (at rest, off the record) to 1 (on the record).
    pub arm: f32,
    /// How far the speed ramp has gone, 0 to 1; `speed` is it eased.
    spin: f32,
    /// How far the arm's swing has gone, 0 to 1; `arm` is it eased.
    swing: f32,
}

/// Eases `t` in 0..=1 in and out.
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Turntable {
    /// Moves everything on by `dt` seconds.
    pub fn step(&mut self, dt: f32, playing: bool, rpm: f32) {
        let dt = dt.max(0.0);
        self.spin = if playing {
            (self.spin + dt / SPIN_UP).min(1.0)
        } else {
            (self.spin - dt / SPIN_DOWN).max(0.0)
        };
        self.swing = if playing {
            (self.swing + dt / ARM).min(1.0)
        } else {
            (self.swing - dt / ARM).max(0.0)
        };
        self.speed = smoothstep(self.spin);
        self.arm = smoothstep(self.swing);
        self.angle = (self.angle + self.speed * rpm / 60.0 * TAU * dt).rem_euclid(TAU);
    }

    /// Whether anything moves, so the window must keep drawing.
    pub fn busy(&self, playing: bool) -> bool {
        playing || self.spin > 0.0 || self.swing > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 120.0;

    fn run(t: &mut Turntable, seconds: f32, playing: bool) {
        let steps = (seconds / DT).round() as usize;
        for _ in 0..steps {
            t.step(DT, playing, 20.0);
        }
    }

    /// The angle travelled from `before` to `after`, the shorter way forward.
    fn travelled(before: f32, after: f32) -> f32 {
        (after - before).rem_euclid(TAU)
    }

    #[test]
    fn full_speed_is_the_rpm_given() {
        for (rpm, turns) in [(20.0, 2.0), (45.0, 4.5), (100.0 / 3.0, 10.0 / 3.0)] {
            let mut t = Turntable::default();
            for _ in 0..(1.0 / DT).round() as usize {
                t.step(DT, true, rpm);
            }
            let mut total = 0.0;
            let mut last = t.angle;
            for _ in 0..(6.0 / DT).round() as usize {
                t.step(DT, true, rpm);
                total += travelled(last, t.angle);
                last = t.angle;
            }
            let expected = turns * TAU;
            assert!(
                (total - expected).abs() < 1e-2,
                "{rpm}: {total} vs {expected}"
            );
            assert!((0.0..TAU).contains(&t.angle));
        }
    }

    #[test]
    fn a_speed_near_a_mark_snaps_to_it() {
        assert_eq!(snap_rpm(33.0), 100.0 / 3.0);
        assert_eq!(snap_rpm(21.0), 20.0);
        assert_eq!(snap_rpm(44.0), 45.0);
        assert_eq!(snap_rpm(60.0), 60.0);
        assert_eq!(snap_rpm(2.0), 10.0);
        assert_eq!(snap_rpm(99.0), 78.0);
    }

    #[test]
    fn play_speeds_up_over_0_6_s() {
        let mut t = Turntable::default();
        run(&mut t, 0.3, true);
        assert!(t.speed > 0.0 && t.speed < 1.0, "{}", t.speed);
        run(&mut t, 0.3, true);
        assert!((t.speed - 1.0).abs() < 1e-4, "{}", t.speed);
    }

    #[test]
    fn pause_slows_down_without_a_jump() {
        let mut t = Turntable::default();
        run(&mut t, 1.0, true);
        let most = 20.0 / 60.0 * TAU * DT * 1.001;
        for _ in 0..(1.0 / DT).round() as usize {
            let before = t.angle;
            t.step(DT, false, 20.0);
            assert!(travelled(before, t.angle) <= most);
        }
        assert_eq!(t.speed, 0.0);
    }

    #[test]
    fn the_arm_moves_on_and_off() {
        let mut t = Turntable::default();
        run(&mut t, 0.5, true);
        assert!((t.arm - 1.0).abs() < 1e-4, "{}", t.arm);
        run(&mut t, 0.5, false);
        assert!(t.arm.abs() < 1e-4, "{}", t.arm);
    }

    #[test]
    fn at_rest_nothing_is_busy() {
        let mut t = Turntable::default();
        assert!(!t.busy(false));
        assert!(t.busy(true));
        run(&mut t, 1.0, true);
        run(&mut t, 1.0, false);
        assert!(!t.busy(false), "settled after a pause: {t:?}");
    }
}
