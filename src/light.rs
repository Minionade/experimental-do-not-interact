//! Chunk-local light propagation (sky + block light) via BFS.
use crate::chunk::{Chunk, HEIGHT, MIN_Y};
use std::collections::VecDeque;

/// How much light a block absorbs (0 transparent .. 15 opaque).
pub fn opacity(name: &str) -> u8 {
    match name {
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "minecraft:glass" | "minecraft:torch" => 0,
        "minecraft:water" | "minecraft:ice" => 2,
        "minecraft:lava" => 1,
        n if n.ends_with("_leaves") => 1,
        _ => 15,
    }
}

pub fn emission(name: &str) -> u8 {
    match name {
        "minecraft:lava" | "minecraft:glowstone" | "minecraft:sea_lantern" => 15,
        "minecraft:torch" => 14,
        _ => 0,
    }
}

fn idx(x: usize, y: usize, z: usize) -> usize { (y * 16 + z) * 16 + x } // y is 0..HEIGHT

pub fn compute(c: &mut Chunk) {
    let n = HEIGHT * 256;
    let mut op = vec![0u8; n];
    let mut sky = vec![0u8; n];
    let mut blk = vec![0u8; n];
    let mut q: VecDeque<(usize, usize, usize)> = VecDeque::new();
    for y in 0..HEIGHT { for z in 0..16 { for x in 0..16 {
        let b = &c.sections[y / 16].palette[c.sections[y / 16].indices[(y % 16) * 256 + z * 16 + x] as usize];
        op[idx(x, y, z)] = opacity(&b.name);
        blk[idx(x, y, z)] = emission(&b.name);
    }}}
    // Sky light: straight down each column, then spread.
    for z in 0..16 { for x in 0..16 {
        let mut l = 15u8;
        for y in (0..HEIGHT).rev() {
            l = l.saturating_sub(op[idx(x, y, z)]);
            sky[idx(x, y, z)] = l;
            if l == 0 { break; }
        }
    }}
    for y in 0..HEIGHT { for z in 0..16 { for x in 0..16 {
        if sky[idx(x, y, z)] > 1 { q.push_back((x, y, z)); }
    }}}
    spread(&mut sky, &op, &mut q);
    for y in 0..HEIGHT { for z in 0..16 { for x in 0..16 {
        if blk[idx(x, y, z)] > 1 { q.push_back((x, y, z)); }
    }}}
    spread(&mut blk, &op, &mut q);
    for (i, s) in c.sections.iter_mut().enumerate() {
        for y in 0..16 { for z in 0..16 { for x in 0..16 {
            let j = y * 256 + z * 16 + x;
            s.sky[j] = sky[idx(x, i * 16 + y, z)];
            s.block_light[j] = blk[idx(x, i * 16 + y, z)];
        }}}
    }
    let _ = MIN_Y;
}

fn spread(l: &mut [u8], op: &[u8], q: &mut VecDeque<(usize, usize, usize)>) {
    while let Some((x, y, z)) = q.pop_front() {
        let cur = l[idx(x, y, z)];
        if cur <= 1 { continue; }
        for (dx, dy, dz) in [(1i32, 0i32, 0i32), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
            if !(0..16).contains(&nx) || !(0..16).contains(&nz) || !(0..HEIGHT as i32).contains(&ny) { continue; }
            let (nx, ny, nz) = (nx as usize, ny as usize, nz as usize);
            if op[idx(nx, ny, nz)] >= 15 { continue; }
            let v = cur.saturating_sub(1 + op[idx(nx, ny, nz)]);
            if v > l[idx(nx, ny, nz)] { l[idx(nx, ny, nz)] = v; q.push_back((nx, ny, nz)); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::BlockState;
    #[test]
    fn sky_and_block_light() {
        let mut c = Chunk::empty(0, 0);
        for x in 0..16 { for z in 0..16 { c.set(x, 10, z, &BlockState::simple("minecraft:stone")); } }
        c.set(8, 5, 8, &BlockState::simple("minecraft:glowstone"));
        compute(&mut c);
        assert_eq!(c.sky_light(3, 50, 3), 15);
        assert_eq!(c.sky_light(3, 9, 3), 0); // under a solid roof
        assert_eq!(c.block_light(8, 5, 8), 15);
        assert_eq!(c.block_light(9, 5, 8), 14);
        assert_eq!(c.block_light(8, 5, 11), 12);
    }
}
