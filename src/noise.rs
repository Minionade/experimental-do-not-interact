//! Improved Perlin noise (Ken Perlin 2002) with octave stacking.
use crate::rng::JavaRandom;

pub struct Perlin { p: [u8; 512], ox: f64, oy: f64, oz: f64 }

impl Perlin {
    pub fn new(rng: &mut JavaRandom) -> Self {
        let (ox, oy, oz) = (rng.next_double() * 256.0, rng.next_double() * 256.0, rng.next_double() * 256.0);
        let mut perm: Vec<u8> = (0..=255).collect();
        for i in 0..256 {
            let j = i + rng.next_int_bound(256 - i as i32) as usize;
            perm.swap(i, j);
        }
        let mut p = [0u8; 512];
        for i in 0..512 { p[i] = perm[i & 255]; }
        Self { p, ox, oy, oz }
    }
    fn grad(h: u8, x: f64, y: f64, z: f64) -> f64 {
        let h = h & 15;
        let u = if h < 8 { x } else { y };
        let v = if h < 4 { y } else if h == 12 || h == 14 { x } else { z };
        (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
    }
    pub fn sample(&self, x: f64, y: f64, z: f64) -> f64 {
        let (x, y, z) = (x + self.ox, y + self.oy, z + self.oz);
        let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
        let (xi, yi, zi) = ((fx as i64 & 255) as usize, (fy as i64 & 255) as usize, (fz as i64 & 255) as usize);
        let (x, y, z) = (x - fx, y - fy, z - fz);
        let fade = |t: f64| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        let (u, v, w) = (fade(x), fade(y), fade(z));
        let p = &self.p;
        let a = p[xi] as usize + yi; let aa = p[a] as usize + zi; let ab = p[a + 1] as usize + zi;
        let b = p[xi + 1] as usize + yi; let ba = p[b] as usize + zi; let bb = p[b + 1] as usize + zi;
        let l = |t: f64, a: f64, b: f64| a + t * (b - a);
        l(w,
          l(v, l(u, Self::grad(p[aa], x, y, z), Self::grad(p[ba], x - 1.0, y, z)),
               l(u, Self::grad(p[ab], x, y - 1.0, z), Self::grad(p[bb], x - 1.0, y - 1.0, z))),
          l(v, l(u, Self::grad(p[aa + 1], x, y, z - 1.0), Self::grad(p[ba + 1], x - 1.0, y, z - 1.0)),
               l(u, Self::grad(p[ab + 1], x, y - 1.0, z - 1.0), Self::grad(p[bb + 1], x - 1.0, y - 1.0, z - 1.0))))
    }
}

pub struct Octaves { layers: Vec<Perlin> }

impl Octaves {
    pub fn new(rng: &mut JavaRandom, n: usize) -> Self {
        Self { layers: (0..n).map(|_| Perlin::new(rng)).collect() }
    }
    /// Sum of octaves, each at double frequency, half amplitude; normalized to about [-1, 1].
    pub fn sample(&self, x: f64, y: f64, z: f64) -> f64 {
        let (mut f, mut a, mut sum, mut norm) = (1.0, 1.0, 0.0, 0.0);
        for l in &self.layers {
            sum += l.sample(x * f, y * f, z * f) * a;
            norm += a; f *= 2.0; a *= 0.5;
        }
        sum / norm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_and_bounded() {
        let a = Octaves::new(&mut JavaRandom::new(1), 4);
        let b = Octaves::new(&mut JavaRandom::new(1), 4);
        for i in 0..200 {
            let (x, z) = (i as f64 * 0.37, i as f64 * 0.11);
            let v = a.sample(x, 0.5, z);
            assert_eq!(v, b.sample(x, 0.5, z));
            assert!(v.abs() <= 1.5);
        }
    }
}
