//! Face-culled chunk meshing with per-face light and a fixed directional shade.
use crate::chunk::{BlockState, Chunk, MIN_Y};
use crate::entity::{Entity, Kind};
use crate::light::{emission, opacity};

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
        "minecraft:lava" => [255, 110, 20], "minecraft:glowstone" => [255, 200, 110],
        "minecraft:sea_lantern" => [200, 230, 220], "minecraft:torch" => [255, 190, 80],
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
            let is_water = matches!(b.name.as_str(), "minecraft:water" | "minecraft:lava");
            // Emissive blocks ignore ambient light and exceed 1.0 so HDR outputs can show them brighter than white.
            let glow = emission(&b.name) as f32 / 15.0 * 3.0;
            let col = block_color(&b.name);
            // Fluid surface height: source 8/9, flowing (8-L)/9, falling or covered columns full height.
            let fluid_h = if is_water {
                let covered = probe(x as i32, y + 1, z as i32).is_some_and(|(a, _)| a.name == b.name);
                Some(if covered { 1.0 } else { match b.fluid_level() { Some(0) | None => 8.0 / 9.0, Some(l) if l >= 8 => 1.0, Some(l) => (8 - l) as f32 / 9.0 } })
            } else { None };
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
                let f = if glow > 0.0 { glow * (0.85 + 0.15 * shade) } else { shade * (0.15 + 0.85 * light as f32 / 15.0) };
                let base = m.vertices.len() as u32;
                for k in corners {
                    let ky = match fluid_h { Some(h) if k[1] == 1.0 => h, _ => k[1] };
                    m.vertices.push(Vertex {
                        pos: [x as f32 + k[0], y as f32 + ky, z as f32 + k[2]],
                        color: [col[0] * f, col[1] * f, col[2] * f],
                    });
                }
                m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }}}
    }
    m
}

/// Axis-aligned box mesh with the fixed directional shading (no light data).
pub fn box_mesh(min: [f32; 3], max: [f32; 3], color: [f32; 3]) -> Mesh {
    let mut m = Mesh::default();
    for (_, shade, corners) in &FACES {
        let base = m.vertices.len() as u32;
        for k in corners {
            m.vertices.push(Vertex {
                pos: [min[0] + k[0] * (max[0] - min[0]), min[1] + k[1] * (max[1] - min[1]), min[2] + k[2] * (max[2] - min[2])],
                color: [color[0] * shade, color[1] * shade, color[2] * shade],
            });
        }
        m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    m
}

fn append(dst: &mut Mesh, src: Mesh, f: impl Fn([f32; 3]) -> [f32; 3]) {
    let base = dst.vertices.len() as u32;
    dst.vertices.extend(src.vertices.into_iter().map(|v| Vertex { pos: f(v.pos), color: v.color }));
    dst.indices.extend(src.indices.into_iter().map(|i| i + base));
}

/// World-space model for an entity (boxes rotated by yaw about the feet).
pub fn entity_model(e: &Entity) -> Mesh {
    let parts: Vec<([f32; 3], [f32; 3], [f32; 3])> = match e.kind {
        // (min, max, colour) in local space: x right, y up, z forward.
        Kind::Pig => {
            let pink = [0.96, 0.66, 0.70];
            let dark = [0.85, 0.50, 0.55];
            vec![([-0.25, 0.30, -0.45], [0.25, 0.75, 0.35], pink),     // body
                 ([-0.22, 0.45, 0.35], [0.22, 0.88, 0.65], pink),      // head
                 ([-0.12, 0.48, 0.65], [0.12, 0.64, 0.72], dark),      // snout
                 ([-0.22, 0.0, -0.40], [-0.08, 0.30, -0.26], pink), ([0.08, 0.0, -0.40], [0.22, 0.30, -0.26], pink),
                 ([-0.22, 0.0, 0.14], [-0.08, 0.30, 0.28], pink), ([0.08, 0.0, 0.14], [0.22, 0.30, 0.28], pink)]
        }
        Kind::Player => vec![([-0.3, 0.0, -0.15], [0.3, 1.8, 0.15], [0.3, 0.5, 0.8])],
        Kind::Item => vec![([-0.125, 0.0, -0.125], [0.125, 0.25, 0.125], [0.9, 0.9, 0.9])],
    };
    let (sy, cy) = (e.yaw.sin() as f32, e.yaw.cos() as f32);
    let origin = [e.pos[0] as f32, e.pos[1] as f32, e.pos[2] as f32];
    let mut out = Mesh::default();
    for (min, max, color) in parts {
        append(&mut out, box_mesh(min, max, color), |p| [
            origin[0] + p[0] * cy + p[2] * sy, origin[1] + p[1], origin[2] - p[0] * sy + p[2] * cy]);
    }
    out
}

/// Wireframe-style outline around a block (12 thin boxes), for block selection.
pub fn outline_mesh(pos: [i32; 3]) -> Mesh {
    let (t, o) = (0.012f32, 0.003f32);
    let (lo, hi) = ([pos[0] as f32 - o, pos[1] as f32 - o, pos[2] as f32 - o], [pos[0] as f32 + 1.0 + o, pos[1] as f32 + 1.0 + o, pos[2] as f32 + 1.0 + o]);
    let mut m = Mesh::default();
    let black = [0.02, 0.02, 0.02];
    for axis in 0..3 {
        for corner in 0..4 {
            let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
            let (mut min, mut max) = (lo, hi);
            for (ax, bit) in [(a, corner & 1), (b, corner >> 1)] {
                if bit == 0 { max[ax] = lo[ax] + t; } else { min[ax] = hi[ax] - t; }
            }
            append(&mut m, box_mesh(min, max, black), |p| p);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fluid_surface_is_lower_than_a_full_block() {
        let mut c = Chunk::empty(0, 0);
        c.set(5, 5, 5, &BlockState::fluid("minecraft:water", 0));
        let m = mesh_chunk(&c, [None; 4]);
        let top = m.vertices.iter().map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        assert!((top - (5.0 + 8.0 / 9.0)).abs() < 1e-6);
    }

    #[test]
    fn entity_model_follows_position_and_yaw() {
        use crate::entity::EntityWorld;
        let mut w = EntityWorld::new(1);
        let id = w.spawn(Kind::Pig, [10.0, 70.0, 20.0]);
        let e = w.get_mut(id).unwrap().clone();
        let m = entity_model(&e);
        assert!(!m.vertices.is_empty());
        assert!(m.vertices.iter().all(|v| (v.pos[0] - 10.0).abs() < 1.0 && (v.pos[2] - 20.0).abs() < 1.0));
        assert_eq!(outline_mesh([1, 2, 3]).indices.len(), 12 * 36);
    }

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
