//! Original terrain generator (not seed-compatible with Java edition).
use crate::chunk::{BlockState, Chunk, MIN_Y};
use crate::noise::Octaves;
use crate::rng::JavaRandom;

pub const SEA_LEVEL: i32 = 63;

pub struct Generator { seed: i64, height: Octaves, detail: Octaves, caves: Octaves }

impl Generator {
    pub fn new(seed: i64) -> Self {
        let mut r = JavaRandom::new(seed);
        Self { seed, height: Octaves::new(&mut r, 6), detail: Octaves::new(&mut r, 4), caves: Octaves::new(&mut r, 3) }
    }
    pub fn seed(&self) -> i64 { self.seed }

    pub fn surface_height(&self, wx: i32, wz: i32) -> i32 {
        let (x, z) = (wx as f64, wz as f64);
        let base = self.height.sample(x / 180.0, 0.0, z / 180.0);
        let det = self.detail.sample(x / 40.0, 0.0, z / 40.0);
        (SEA_LEVEL as f64 + 4.0 + base * 45.0 + det * 8.0) as i32
    }

    pub fn generate(&self, cx: i32, cz: i32) -> Chunk {
        let (stone, dirt, grass, water, sand, bedrock) = (
            BlockState::simple("minecraft:stone"), BlockState::simple("minecraft:dirt"),
            BlockState::simple("minecraft:grass_block"), BlockState::simple("minecraft:water"),
            BlockState::simple("minecraft:sand"), BlockState::simple("minecraft:bedrock"));
        let mut c = Chunk::empty(cx, cz);
        for lx in 0..16usize {
            for lz in 0..16usize {
                let (wx, wz) = (cx * 16 + lx as i32, cz * 16 + lz as i32);
                let h = self.surface_height(wx, wz);
                for y in MIN_Y..=h.max(SEA_LEVEL) {
                    let b = if y == MIN_Y { &bedrock }
                        else if y > h { &water }
                        else if y == h { if h <= SEA_LEVEL + 1 { &sand } else { &grass } }
                        else if y >= h - 3 { &dirt }
                        else {
                            let cave = self.caves.sample(wx as f64 / 30.0, y as f64 / 20.0, wz as f64 / 30.0);
                            if cave > 0.45 && y < h - 6 { continue } else { &stone }
                        };
                    c.set(lx, y, lz, b);
                }
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_chunk() {
        let g = Generator::new(12345);
        let (a, b) = (g.generate(2, -3), Generator::new(12345).generate(2, -3));
        for y in [MIN_Y, 0, 60, 100] {
            assert_eq!(a.get(5, y, 7), b.get(5, y, 7));
        }
        assert_eq!(a.get(0, MIN_Y, 0).name, "minecraft:bedrock");
    }
}
