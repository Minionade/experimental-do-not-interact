//! Generates a region of terrain in parallel and writes a top-down PPM map.
use blockengine::chunk::Chunk;
use blockengine::worldgen::Generator;
use rayon::prelude::*;
use std::{io::Write, time::Instant};

fn color(name: &str) -> [u8; 3] {
    match name {
        "minecraft:grass_block" => [95, 159, 53], "minecraft:sand" => [219, 207, 142],
        "minecraft:water" => [50, 80, 200], "minecraft:stone" => [125, 125, 125],
        "minecraft:dirt" => [134, 96, 67], _ => [0, 0, 0],
    }
}

fn main() {
    let seed: i64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let n = 16; // n x n chunks
    let g = Generator::new(seed);
    let t = Instant::now();
    let chunks: Vec<Chunk> = (0..n * n).into_par_iter().map(|i| g.generate(i % n - n / 2, i / n - n / 2)).collect();
    eprintln!("generated {} chunks in {:?}", chunks.len(), t.elapsed());
    let w = (n * 16) as usize;
    let mut img = vec![0u8; w * w * 3];
    for c in &chunks {
        for lx in 0..16 { for lz in 0..16 {
            let y = (-64..320).rev().find(|&y| !c.get(lx, y, lz).is_air()).unwrap_or(-64);
            let px = ((c.x + n / 2) * 16) as usize + lx;
            let pz = ((c.z + n / 2) * 16) as usize + lz;
            img[(pz * w + px) * 3..][..3].copy_from_slice(&color(&c.get(lx, y, lz).name));
        }}
    }
    let mut f = std::fs::File::create("map.ppm").unwrap();
    write!(f, "P6\n{w} {w}\n255\n").unwrap();
    f.write_all(&img).unwrap();
}
