//! Original terrain generator (not seed-compatible with Java edition).
use crate::chunk::{BlockState, Chunk, HEIGHT, MIN_Y};
use crate::light;
use crate::noise::Octaves;
use crate::rng::JavaRandom;

pub const SEA_LEVEL: i32 = 63;

pub struct Generator {
    seed: i64,
    height: Octaves,
    detail: Octaves,
    caves: Octaves,
    temp: Octaves,
    humid: Octaves,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Biome { Ocean, Plains, Forest, Desert, Snowy, Windswept }

impl Biome {
    pub fn id(self) -> &'static str {
        match self {
            Biome::Ocean => "minecraft:ocean", Biome::Plains => "minecraft:plains",
            Biome::Forest => "minecraft:forest", Biome::Desert => "minecraft:desert",
            Biome::Snowy => "minecraft:snowy_plains", Biome::Windswept => "minecraft:windswept_hills",
        }
    }
}

fn hash(seed: i64, x: i32, z: i32) -> u64 {
    let mut h = (seed as u64) ^ (x as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ (z as u64).wrapping_mul(0xC2B2AE3D27D4EB4F);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D049BB133111EB);
    h ^ (h >> 31)
}

impl Generator {
    pub fn new(seed: i64) -> Self {
        let mut r = JavaRandom::new(seed);
        Self { seed, height: Octaves::new(&mut r, 6), detail: Octaves::new(&mut r, 4), caves: Octaves::new(&mut r, 3),
               temp: Octaves::new(&mut r, 3), humid: Octaves::new(&mut r, 3) }
    }
    pub fn seed(&self) -> i64 { self.seed }

    pub fn surface_height(&self, wx: i32, wz: i32) -> i32 {
        let (x, z) = (wx as f64, wz as f64);
        let base = self.height.sample(x / 180.0, 0.0, z / 180.0);
        let det = self.detail.sample(x / 40.0, 0.0, z / 40.0);
        (SEA_LEVEL as f64 + 4.0 + base * 45.0 + det * 8.0) as i32
    }

    pub fn biome(&self, wx: i32, wz: i32, h: i32) -> Biome {
        let (x, z) = (wx as f64, wz as f64);
        let t = self.temp.sample(x / 400.0, 0.0, z / 400.0) * 2.0;
        let m = self.humid.sample(x / 300.0, 0.0, z / 300.0) * 2.0;
        if h < SEA_LEVEL - 1 { Biome::Ocean }
        else if h > 110 { Biome::Windswept }
        else if t > 0.35 && m < 0.1 { Biome::Desert }
        else if t < -0.35 { Biome::Snowy }
        else if m > 0.15 { Biome::Forest }
        else { Biome::Plains }
    }

    /// Tree origin at a world column, if any. Pure function of (seed, column) so trees line up across chunks.
    fn tree_at(&self, wx: i32, wz: i32, h: i32, b: Biome) -> Option<i32> {
        let rate = match b { Biome::Forest => 28, Biome::Plains => 400, _ => return None };
        if h <= SEA_LEVEL + 1 || hash(self.seed, wx, wz) % rate != 0 { return None; }
        Some(4 + (hash(self.seed ^ 1, wx, wz) % 3) as i32)
    }

    pub fn generate(&self, cx: i32, cz: i32) -> Chunk {
        let blk = |n: &str| BlockState::simple(n);
        let (stone, dirt, grass, water, sand, bedrock, snow, sandstone) = (
            blk("minecraft:stone"), blk("minecraft:dirt"), blk("minecraft:grass_block"), blk("minecraft:water"),
            blk("minecraft:sand"), blk("minecraft:bedrock"), blk("minecraft:snow_block"), blk("minecraft:sandstone"));
        let lava = blk("minecraft:lava");
        let log = BlockState { name: "minecraft:oak_log".into(), props: "axis=y".into() };
        let leaves = BlockState { name: "minecraft:oak_leaves".into(), props: "distance=1,persistent=false,waterlogged=false".into() };

        // Heights/biomes for chunk plus a 2-block margin (for trees spilling in).
        const M: i32 = 2;
        let w = (16 + 2 * M) as usize;
        let mut hs = vec![0i32; w * w];
        let mut bs = vec![Biome::Plains; w * w];
        for i in 0..w { for j in 0..w {
            let (wx, wz) = (cx * 16 + i as i32 - M, cz * 16 + j as i32 - M);
            let h = self.surface_height(wx, wz);
            hs[i * w + j] = h;
            bs[i * w + j] = self.biome(wx, wz, h);
        }}
        let at = |lx: i32, lz: i32| ((lx + M) as usize) * w + (lz + M) as usize;

        let mut c = Chunk::empty(cx, cz);
        for lx in 0..16usize { for lz in 0..16usize {
            let (wx, wz) = (cx * 16 + lx as i32, cz * 16 + lz as i32);
            let (h, b) = (hs[at(lx as i32, lz as i32)], bs[at(lx as i32, lz as i32)]);
            let (top, filler) = match b {
                Biome::Desert => (&sand, &sandstone),
                Biome::Ocean => (&sand, &sand),
                Biome::Snowy => (&snow, &dirt),
                Biome::Windswept if h > 120 => (&stone, &stone),
                _ => if h <= SEA_LEVEL + 1 { (&sand, &sand) } else { (&grass, &dirt) },
            };
            for y in MIN_Y..=h.max(SEA_LEVEL) {
                let blkref = if y == MIN_Y { &bedrock }
                    else if y > h { &water }
                    else if y == h { top }
                    else if y >= h - 3 { filler }
                    else {
                        let cave = self.caves.sample(wx as f64 / 30.0, y as f64 / 20.0, wz as f64 / 30.0);
                        if cave > 0.45 && y < h - 6 { if y <= -54 { &lava } else { continue } } else { &stone }
                    };
                c.set(lx, y, lz, blkref);
            }
            for y in (MIN_Y..MIN_Y + HEIGHT as i32).step_by(4) { c.set_biome(lx, y, lz, b.id()); }
        }}

        // Ores: random blobs clipped to the chunk, replacing stone only.
        let mut r = JavaRandom::new(self.seed ^ (cx as i64).wrapping_mul(341873128712) ^ (cz as i64).wrapping_mul(132897987541));
        for (name, tries, ymin, ymax, size) in [
            ("minecraft:coal_ore", 20, 0, 128, 8), ("minecraft:iron_ore", 14, -32, 64, 6),
            ("minecraft:gold_ore", 4, -64, 30, 5), ("minecraft:diamond_ore", 2, -64, 14, 3)] {
            let ore = blk(name);
            for _ in 0..tries {
                let (ox, oz) = (r.next_int_bound(16), r.next_int_bound(16));
                let oy = ymin + r.next_int_bound(ymax - ymin);
                for _ in 0..size {
                    let (x, z) = ((ox + r.next_int_bound(3) - 1).clamp(0, 15) as usize, (oz + r.next_int_bound(3) - 1).clamp(0, 15) as usize);
                    let y = oy + r.next_int_bound(3) - 1;
                    if y > MIN_Y && c.get(x, y, z) == &stone { c.set(x, y, z, &ore); }
                }
            }
        }

        // Trees from every origin within the margin.
        for tx in -M..16 + M { for tz in -M..16 + M {
            let (h, b) = (hs[at(tx, tz)], bs[at(tx, tz)]);
            let (wx, wz) = (cx * 16 + tx, cz * 16 + tz);
            let Some(th) = self.tree_at(wx, wz, h, b) else { continue };
            for dy in 1..=th + 1 {
                let y = h + dy;
                let r_: i32 = if dy >= th - 1 { if dy > th { 1 } else { 2 } } else { 0 };
                if r_ == 0 {
                    if (0..16).contains(&tx) && (0..16).contains(&tz) && dy <= th { c.set(tx as usize, y, tz as usize, &log); }
                    continue;
                }
                for dx in -r_..=r_ { for dz in -r_..=r_ {
                    if dx.abs() == r_ && dz.abs() == r_ && (dy > th || hash(self.seed ^ 2, wx + dx, wz + dz + dy) % 2 == 0) { continue }
                    let (lx, lz) = (tx + dx, tz + dz);
                    if (0..16).contains(&lx) && (0..16).contains(&lz) && c.get(lx as usize, y, lz as usize).is_air() {
                        c.set(lx as usize, y, lz as usize, &leaves);
                    }
                }}
            }
        }}

        light::compute(&mut c);
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
        for y in [MIN_Y, 0, 60, 100] { assert_eq!(a.get(5, y, 7), b.get(5, y, 7)); }
        assert_eq!(a.get(0, MIN_Y, 0).name, "minecraft:bedrock");
    }
    #[test]
    fn trees_and_ores_appear() {
        let g = Generator::new(3);
        let (mut logs, mut ores) = (0, 0);
        for cx in -6..6 { for cz in -6..6 {
            let c = g.generate(cx, cz);
            for s in &c.sections { for p in &s.palette {
                if p.name == "minecraft:oak_log" { logs += 1 }
                if p.name.ends_with("_ore") { ores += 1 }
            }}
        }}
        assert!(logs > 0, "no trees");
        assert!(ores > 0, "no ores");
    }
}
