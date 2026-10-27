//! Chunk-local light propagation (sky + block light) via BFS.
use crate::access::{BlockAccess, ChunkMap};
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

#[derive(Clone, Copy)]
enum Kind { Sky, Block }

fn level(chunks: &ChunkMap, wx: i32, y: i32, wz: i32, k: Kind) -> u8 {
    let c = &chunks[&(wx >> 4, wz >> 4)];
    let (lx, lz) = ((wx & 15) as usize, (wz & 15) as usize);
    match k { Kind::Sky => c.sky_light(lx, y, lz), Kind::Block => c.block_light(lx, y, lz) }
}

fn set_level(chunks: &mut ChunkMap, wx: i32, y: i32, wz: i32, k: Kind, v: u8) {
    let c = chunks.get_mut(&(wx >> 4, wz >> 4)).unwrap();
    let ry = (y - MIN_Y) as usize;
    let i = (ry % 16) * 256 + (wz & 15) as usize * 16 + (wx & 15) as usize;
    let s = &mut c.sections[ry / 16];
    match k { Kind::Sky => s.sky[i] = v, Kind::Block => s.block_light[i] = v }
}

/// Carries light across chunk borders in the 3x3 neighbourhood of (cx, cz).
/// Each chunk must already have been through `compute`. Call after a chunk's neighbours load.
pub fn stitch(chunks: &mut ChunkMap, cx: i32, cz: i32) {
    let region = |x: i32, z: i32| ((x >> 4) - cx).abs() <= 1 && ((z >> 4) - cz).abs() <= 1;
    for k in [Kind::Sky, Kind::Block] {
        let mut q: VecDeque<(i32, i32, i32)> = VecDeque::new();
        for dx in -1..=1 { for dz in -1..=1 {
            if !chunks.contains_key(&(cx + dx, cz + dz)) { continue; }
            for y in MIN_Y..MIN_Y + HEIGHT as i32 { for i in 0..16 { for e in [0, 15] {
                for (lx, lz) in [(e, i), (i, e)] {
                    let (wx, wz) = ((cx + dx) * 16 + lx, (cz + dz) * 16 + lz);
                    if level(chunks, wx, y, wz, k) > 1 { q.push_back((wx, y, wz)); }
                }
            }}}
        }}
        while let Some((x, y, z)) = q.pop_front() {
            let cur = level(chunks, x, y, z, k);
            if cur <= 1 { continue; }
            for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                if !region(nx, nz) || !chunks.contains_key(&(nx >> 4, nz >> 4)) { continue; }
                let Some(b) = chunks.block(nx, ny, nz) else { continue };
                let op = opacity(&b.name);
                if op >= 15 { continue; }
                let v = cur.saturating_sub(1 + op);
                if v > level(chunks, nx, ny, nz, k) { set_level(chunks, nx, ny, nz, k, v); q.push_back((nx, ny, nz)); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::BlockState;
    #[test]
    fn light_crosses_chunk_borders() {
        let mut m = ChunkMap::new();
        for x in 0..2 { m.insert((x, 0), Chunk::empty(x, 0)); }
        m.set_block(15, 5, 8, &BlockState::simple("minecraft:glowstone"));
        for c in m.values_mut() { compute(c); }
        assert_eq!(m[&(1, 0)].block_light(0, 5, 8), 0, "chunk-local pass can't see across the border");
        stitch(&mut m, 0, 0);
        assert_eq!(m[&(1, 0)].block_light(0, 5, 8), 14);
        assert_eq!(m[&(1, 0)].block_light(4, 5, 8), 10);
        // Sky light also leaks sideways under an overhang across the border.
        let mut m = ChunkMap::new();
        for x in 0..2 { m.insert((x, 0), Chunk::empty(x, 0)); }
        for x in 16..32 { for z in 0..16 { m.set_block(x, 20, z, &BlockState::simple("minecraft:stone")); } }
        for c in m.values_mut() { compute(c); }
        assert_eq!(m[&(1, 0)].sky_light(0, 19, 8), 0);
        stitch(&mut m, 0, 0);
        assert_eq!(m[&(1, 0)].sky_light(0, 19, 8), 14);
    }

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
