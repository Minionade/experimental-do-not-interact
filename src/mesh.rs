//! Face-culled chunk meshing with per-face light and a fixed directional shade.
use crate::chunk::{BlockState, Chunk, MIN_Y};
use crate::light::opacity;

#[derive(Clone, Copy, Debug)]
pub struct Vertex { pub pos: [f32; 3], pub color: [f32; 3] }

#[derive(Default)]
pub struct Mesh { pub vertices: Vec<Vertex>, pub indices: Vec<u32> }

pub fn block_color(name: &str) -> [f32; 3] {
    let c = match name {
        "minecraft:grass_block" => [95, 159, 53], "minecraft:sand" => [219, 207, 142],
        "minecraft:water" => [50, 80, 200], "minecraft:stone" => [125, 125, 125],
        "minecraft:dirt" => [134, 96, 67], "minecraft:bedrock" => [50, 50, 50],
        "minecraft:snow_block" => [245, 250, 250], "minecraft:sandstone" => [216, 203, 155],
        "minecraft:oak_log" => [102, 81, 50], "minecraft:oak_leaves" => [60, 120, 40],
        "minecraft:coal_ore" => [60, 60, 60], "minecraft:iron_ore" => [190, 150, 120],
        "minecraft:gold_ore" => [230, 200, 60], "minecraft:diamond_ore" => [90, 220, 220],
        _ => [200, 0, 200],
    };
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]
}

/// Direction, shade, and the 4 corner offsets (CCW seen from outside).
const FACES: [([i32; 3], f32, [[f32; 3]; 4]); 6] = [
    ([1, 0, 0], 0.8, [[1., 0., 1.], [1., 0., 0.], [1., 1., 0.], [1., 1., 1.]]),
    ([-1, 0, 0], 0.8, [[0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]]),
    ([0, 1, 0], 1.0, [[0., 1., 1.], [1., 1., 1.], [1., 1., 0.], [0., 1., 0.]]),
    ([0, -1, 0], 0.5, [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]]),
    ([0, 0, 1], 0.65, [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]]),
    ([0, 0, -1], 0.65, [[1., 0., 0.], [0., 0., 0.], [0., 1., 0.], [1., 1., 0.]]),
];

/// `nb` neighbours in order [-x, +x, -z, +z]; missing neighbours are treated as open air.
pub fn mesh_chunk(c: &Chunk, nb: [Option<&Chunk>; 4]) -> Mesh {
    let mut m = Mesh::default();
    let top = MIN_Y + (c.sections.len() * 16) as i32;
    // Returns (block, light) at chunk-local coords possibly outside the chunk horizontally.
    let probe = |x: i32, y: i32, z: i32| -> Option<(&BlockState, u8)> {
        if y < MIN_Y || y >= top { return None; }
        let (ch, lx, lz) = if x < 0 { (nb[0], x + 16, z) } else if x > 15 { (nb[1], x - 16, z) }
            else if z < 0 { (nb[2], x, z + 16) } else if z > 15 { (nb[3], x, z - 16) } else { (Some(c), x, z) };
        let ch = ch?;
        let (lx, lz) = (lx as usize, lz as usize);
        Some((ch.get(lx, y, lz), ch.sky_light(lx, y, lz).max(ch.block_light(lx, y, lz))))
    };
    for (si, s) in c.sections.iter().enumerate() {
        if s.palette.len() == 1 && s.palette[0].is_air() { continue; }
        for ly in 0..16usize { for z in 0..16usize { for x in 0..16usize {
            let b = s.get(x, ly, z);
            if b.is_air() { continue; }
            let y = MIN_Y + (si * 16 + ly) as i32;
            let is_water = b.name == "minecraft:water";
            let col = block_color(&b.name);
            for (dir, shade, corners) in &FACES {
                let (nx, ny, nz) = (x as i32 + dir[0], y + dir[1], z as i32 + dir[2]);
                let light = match probe(nx, ny, nz) {
                    Some((nbk, l)) => {
                        let hidden = if is_water { nbk.name == b.name || opacity(&nbk.name) >= 15 }
                            else { opacity(&nbk.name) >= 15 };
                        if hidden { continue; }
                        l
                    }
                    None => 15,
                };
                let f = shade * (0.15 + 0.85 * light as f32 / 15.0);
                let base = m.vertices.len() as u32;
                for k in corners {
                    m.vertices.push(Vertex {
                        pos: [x as f32 + k[0], y as f32 + k[1], z as f32 + k[2]],
                        color: [col[0] * f, col[1] * f, col[2] * f],
                    });
                }
                m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }}}
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_block_has_six_faces_and_buried_faces_are_culled() {
        let mut c = Chunk::empty(0, 0);
        c.set(5, 5, 5, &BlockState::simple("minecraft:stone"));
        assert_eq!(mesh_chunk(&c, [None; 4]).indices.len(), 6 * 6);
        c.set(6, 5, 5, &BlockState::simple("minecraft:stone"));
        // two adjacent cubes: 12 faces minus 2 shared = 10
        assert_eq!(mesh_chunk(&c, [None; 4]).indices.len(), 10 * 6);
    }
}
