//! Small numeric helpers shared across the simulation.

use macroquad::prelude::*;

/// Linear interpolation. `t` is not clamped.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Hermite smoothstep over `[0, 1]`. Used by the terrain noise so interpolated
/// control points meet with matching slope instead of visible creases.
#[inline]
pub fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Maps `v` from `[in_lo, in_hi]` onto `[out_lo, out_hi]`, clamped at both ends.
#[inline]
pub fn remap(v: f32, in_lo: f32, in_hi: f32, out_lo: f32, out_hi: f32) -> f32 {
    if (in_hi - in_lo).abs() < f32::EPSILON {
        return out_lo;
    }
    let t = ((v - in_lo) / (in_hi - in_lo)).clamp(0.0, 1.0);
    lerp(out_lo, out_hi, t)
}

/// Moves `current` toward `target` by at most `max_delta`. Frame-rate independent
/// when `max_delta` is a rate multiplied by `dt`, and unlike an exponential ease
/// it actually arrives.
#[inline]
pub fn approach(current: f32, target: f32, max_delta: f32) -> f32 {
    let d = target - current;
    if d.abs() <= max_delta {
        target
    } else {
        current + d.signum() * max_delta
    }
}

/// Exponential decay toward zero that behaves identically at any frame rate.
/// `retain` is the fraction surviving after one full second.
#[inline]
pub fn damp(value: f32, retain_per_second: f32, dt: f32) -> f32 {
    value * retain_per_second.powf(dt)
}

/// Wraps an angle into `(-PI, PI]`.
#[inline]
pub fn wrap_angle(a: f32) -> f32 {
    let mut a = a;
    while a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    while a <= -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

/// Rotates `from` toward `to` by at most `max_step` radians, taking the short way
/// round. This is what gives homing missiles a finite turning circle.
#[inline]
pub fn turn_toward(from: f32, to: f32, max_step: f32) -> f32 {
    let delta = wrap_angle(to - from);
    if delta.abs() <= max_step {
        to
    } else {
        wrap_angle(from + delta.signum() * max_step)
    }
}

/// Squared-distance circle overlap test. Avoids the square root.
#[inline]
pub fn circles_overlap(a: Vec2, ra: f32, b: Vec2, rb: f32) -> bool {
    let r = ra + rb;
    a.distance_squared(b) <= r * r
}

/// A colour with its alpha multiplied. Used constantly by the glow renderer.
#[inline]
pub fn fade(c: Color, alpha: f32) -> Color {
    Color::new(c.r, c.g, c.b, c.a * alpha)
}

/// Blends two colours in straight RGB.
#[inline]
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::new(
        lerp(a.r, b.r, t),
        lerp(a.g, b.g, t),
        lerp(a.b, b.b, t),
        lerp(a.a, b.a, t),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approach_arrives_exactly() {
        assert_eq!(approach(0.0, 10.0, 3.0), 3.0);
        assert_eq!(approach(9.0, 10.0, 3.0), 10.0);
        assert_eq!(approach(10.0, 10.0, 3.0), 10.0);
        assert_eq!(approach(20.0, 10.0, 3.0), 17.0);
    }

    #[test]
    fn remap_clamps_outside_the_input_range() {
        assert_eq!(remap(-5.0, 0.0, 10.0, 100.0, 200.0), 100.0);
        assert_eq!(remap(15.0, 0.0, 10.0, 100.0, 200.0), 200.0);
        assert_eq!(remap(5.0, 0.0, 10.0, 100.0, 200.0), 150.0);
    }

    #[test]
    fn remap_survives_a_degenerate_input_range() {
        assert_eq!(remap(3.0, 2.0, 2.0, 7.0, 9.0), 7.0);
    }

    #[test]
    fn wrap_angle_normalises() {
        use std::f32::consts::{PI, TAU};
        assert!((wrap_angle(3.0 * PI) - PI).abs() < 1e-4);
        assert!(wrap_angle(TAU * 5.0).abs() < 1e-3);
        assert!(wrap_angle(-3.0 * PI - 0.1) <= PI);
    }

    #[test]
    fn turn_toward_takes_the_short_way_round() {
        use std::f32::consts::PI;
        // From just under +PI to just over -PI is a short hop across the seam,
        // not a trip most of the way round the circle.
        let out = turn_toward(PI - 0.1, -PI + 0.1, 0.5);
        assert!((wrap_angle(out - (-PI + 0.1))).abs() < 1e-4, "got {out}");
    }

    #[test]
    fn damp_is_framerate_independent() {
        // One second in one step must match one second in a hundred steps.
        let one_step = damp(1.0, 0.5, 1.0);
        let mut many = 1.0;
        for _ in 0..100 {
            many = damp(many, 0.5, 0.01);
        }
        assert!((one_step - many).abs() < 1e-4);
    }

    #[test]
    fn circle_overlap_matches_the_naive_test() {
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(3.0, 4.0); // distance 5
        assert!(circles_overlap(a, 2.0, b, 3.0));
        assert!(!circles_overlap(a, 2.0, b, 2.9));
    }
}
