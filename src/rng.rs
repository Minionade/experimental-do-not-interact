//! Documented PRNGs: java.util.Random (48-bit LCG) and Xoroshiro128++.

pub struct JavaRandom { seed: i64 }

const MULT: i64 = 0x5DEECE66D;
const MASK: i64 = (1 << 48) - 1;

impl JavaRandom {
    pub fn new(seed: i64) -> Self { Self { seed: (seed ^ MULT) & MASK } }
    fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(MULT).wrapping_add(0xB)) & MASK;
        (self.seed >> (48 - bits)) as i32
    }
    pub fn next_int(&mut self) -> i32 { self.next(32) }
    pub fn next_int_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0);
        if bound & (bound - 1) == 0 {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 { return val; }
        }
    }
    pub fn next_long(&mut self) -> i64 {
        ((self.next(32) as i64) << 32).wrapping_add(self.next(32) as i64)
    }
    pub fn next_double(&mut self) -> f64 {
        (((self.next(26) as i64) << 27) + self.next(27) as i64) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

pub struct Xoroshiro { lo: u64, hi: u64 }

impl Xoroshiro {
    pub fn new(lo: u64, hi: u64) -> Self {
        if lo | hi == 0 { Self { lo: 0x9E3779B97F4A7C15, hi: 0x6A09E667F3BCC909 } } else { Self { lo, hi } }
    }
    /// Seeds via splitmix64 stafford-mix as in the reference xoroshiro seeding scheme.
    pub fn from_seed(seed: u64) -> Self {
        let a = seed ^ 0x6A09E667F3BCC909;
        let b = a.wrapping_add(0x9E3779B97F4A7C15);
        let mix = |mut z: u64| { z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB); z ^ (z >> 31) };
        Self::new(mix(a), mix(b))
    }
    pub fn next_u64(&mut self) -> u64 {
        let (s0, mut s1) = (self.lo, self.hi);
        let r = s0.wrapping_add(s1).rotate_left(17).wrapping_add(s0);
        s1 ^= s0;
        self.lo = s0.rotate_left(49) ^ s1 ^ (s1 << 21);
        self.hi = s1.rotate_left(28);
        r
    }
    pub fn next_f64(&mut self) -> f64 { (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn java_random_known_values() {
        // Well-known: new Random(42).nextInt() == -1170105035, then 234785527.
        let mut r = JavaRandom::new(42);
        assert_eq!(r.next_int(), -1170105035);
        assert_eq!(r.next_int(), 234785527);
    }
}
