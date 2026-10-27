//! Day/night cycle: 24000 ticks per day (20 ticks per second, so 20 real minutes). Tick 0 is sunrise,
//! 6000 noon, 12000 sunset, 18000 midnight.
use std::f32::consts::TAU;

pub const DAY_TICKS: f64 = 24000.0;

#[derive(Clone, Copy, Debug)]
pub struct Sky {
    /// 1 in full daylight, 0 at night; scales sky light in the shader.
    pub daylight: f32,
    /// Sky/fog colour (sRGB-authored).
    pub color: [f32; 3],
    /// Unit direction toward the sun and moon.
    pub sun: [f32; 3],
    pub moon: [f32; 3],
    /// 0 by day, 1 at night: star visibility.
    pub stars: f32,
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 { let t = ((x - a) / (b - a)).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] { [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t] }
fn norm(v: [f32; 3]) -> [f32; 3] { let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt(); [v[0] / l, v[1] / l, v[2] / l] }

pub fn sky_at(time: f64) -> Sky {
    let t = time.rem_euclid(DAY_TICKS);
    let angle = ((t - 6000.0) / DAY_TICKS) as f32 * TAU;
    let elev = angle.cos(); // sun height: 1 at noon, -1 at midnight
    // The sun rises in the east (+x), crosses overhead and sets in the west, with a slight southward tilt.
    let sun = norm([-angle.sin(), angle.cos(), 0.25]);
    let moon = norm([angle.sin(), -angle.cos(), -0.25]);
    let daylight = smoothstep(-0.18, 0.25, elev);
    let day = [0.47, 0.66, 1.0];
    let night = [0.012, 0.016, 0.05];
    let mut color = lerp3(night, day, daylight);
    // Warm tint near sunrise and sunset.
    let dusk = (1.0 - elev.abs() / 0.3).clamp(0.0, 1.0);
    color = lerp3(color, [0.95, 0.52, 0.32], dusk * 0.4);
    Sky { daylight, color, sun, moon, stars: smoothstep(0.05, -0.2, elev) }
}

/// Deterministic star field: unit vectors spread over the sphere.
pub fn stars(count: usize, seed: u64) -> Vec<[f32; 3]> {
    let mut s = seed | 1;
    let mut next = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; (s >> 11) as f64 / (1u64 << 53) as f64 };
    (0..count).map(|_| {
        let (z, a) = (next() * 2.0 - 1.0, next() * std::f64::consts::TAU);
        let r = (1.0 - z * z).sqrt();
        [(r * a.cos()) as f32, z as f32, (r * a.sin()) as f32]
    }).collect()
}

/// Stars turn with the sky about the east-west axis.
pub fn rotate_star(v: [f32; 3], time: f64) -> [f32; 3] {
    let a = (time.rem_euclid(DAY_TICKS) / DAY_TICKS) as f32 * TAU;
    let (s, c) = a.sin_cos();
    [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noon_is_bright_midnight_is_dark_and_dawn_is_between() {
        let (noon, dawn, dusk, mid) = (sky_at(6000.0), sky_at(0.0), sky_at(12000.0), sky_at(18000.0));
        assert!((noon.daylight - 1.0).abs() < 1e-6 && noon.stars < 1e-6);
        assert!(mid.daylight < 1e-6 && (mid.stars - 1.0).abs() < 1e-6);
        assert!(dawn.daylight > 0.3 && dawn.daylight < 1.0, "sunrise is partly lit: {}", dawn.daylight);
        assert!((dawn.daylight - dusk.daylight).abs() < 1e-5, "dawn and dusk are symmetric");
        assert!(noon.color[2] > mid.color[2] * 5.0);
    }

    #[test]
    fn sun_rises_east_peaks_overhead_and_sets_west_opposite_the_moon() {
        assert!(sky_at(6000.0).sun[1] > 0.95);
        assert!(sky_at(0.0).sun[0] > 0.9 && sky_at(0.0).sun[1].abs() < 0.3, "east at sunrise");
        assert!(sky_at(12000.0).sun[0] < -0.9, "west at sunset");
        assert!(sky_at(18000.0).sun[1] < -0.95, "below the world at midnight");
        for t in [0.0, 3000.0, 9000.0, 15000.0] {
            let s = sky_at(t);
            assert!(s.sun[0] * s.moon[0] + s.sun[1] * s.moon[1] + s.sun[2] * s.moon[2] < -0.8, "moon opposes the sun at {t}");
            assert!((s.sun.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 1e-5);
        }
        assert_eq!(sky_at(30000.0).sun, sky_at(6000.0).sun, "time wraps every 24000 ticks");
    }

    #[test]
    fn stars_are_deterministic_unit_vectors_that_rotate() {
        let a = stars(100, 7);
        assert_eq!(a, stars(100, 7));
        assert_ne!(a, stars(100, 8));
        assert!(a.iter().all(|v| (v.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-4));
        let r = rotate_star([0.0, 1.0, 0.0], 6000.0);
        assert!((r[0]).abs() < 1e-6 && r[1].abs() < 1e-5 && (r[2] - 1.0).abs() < 1e-5, "quarter day = quarter turn: {r:?}");
        assert_eq!(rotate_star([0.3, 0.2, 0.1], 0.0), [0.3, 0.2, 0.1]);
    }
}
