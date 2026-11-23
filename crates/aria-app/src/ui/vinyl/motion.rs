//! How the record and the tonearm move: the record turns at 33⅓ rpm while playing, speeding up
//! and slowing down smoothly, and the arm swings on and off the record.

use std::f32::consts::TAU;

/// The record's speed, in turns per minute.
pub const RPM: f32 = 100.0 / 3.0;
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
    /// The record's speed, from 0 (still) to 1 (33⅓ rpm).
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
    pub fn step(&mut self, dt: f32, playing: bool) {
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
        self.angle = (self.angle + self.speed * RPM / 60.0 * TAU * dt).rem_euclid(TAU);
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
            t.step(DT, playing);
        }
    }

    /// The angle travelled from `before` to `after`, the shorter way forward.
    fn travelled(before: f32, after: f32) -> f32 {
        (after - before).rem_euclid(TAU)
    }

    #[test]
    fn full_speed_is_33_and_a_third_rpm() {
        let mut t = Turntable::default();
        run(&mut t, 1.0, true);
        let mut total = 0.0;
        let mut last = t.angle;
        for _ in 0..(6.0 / DT).round() as usize {
            t.step(DT, true);
            total += travelled(last, t.angle);
            last = t.angle;
        }
        let expected = 6.0 * RPM / 60.0 * TAU;
        assert!((total - expected).abs() < 1e-2, "{total} vs {expected}");
        assert!((0.0..TAU).contains(&t.angle));
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
        let most = RPM / 60.0 * TAU * DT * 1.001;
        for _ in 0..(1.0 / DT).round() as usize {
            let before = t.angle;
            t.step(DT, false);
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
