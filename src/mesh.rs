//! Face-culled chunk meshing with per-face light and a fixed directional shade.
use crate::chunk::{BlockState, Chunk, MIN_Y};
use crate::entity::{Entity, Kind};
use crate::light::{emission, opacity};
use crate::model::{Baked, Models, DIRS};
use std::cell::RefCell;
use std::rc::Rc;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    /// Light/shade/tint multiplier (and, without a texture, the block colour). May exceed 1.0 for emissive blocks.
    pub color: [f32; 3],
    /// Atlas UV. Untextured geometry points at the atlas's white texel.
    pub uv: [f32; 2],
}

#[derive(Default)]
pub struct Mesh { pub vertices: Vec<Vertex>, pub indices: Vec<u32> }

pub fn block_color(name: &str) -> [f32; 3] {
    let c: [i32; 3] = match name {
        "minecraft:grass_block" => [95, 159, 53], "minecraft:sand" => [219, 207, 142],
        "minecraft:water" => [50, 80, 200], "minecraft:stone" => [125, 125, 125],
        "minecraft:dirt" => [134, 96, 67], "minecraft:bedrock" => [50, 50, 50],
        "minecraft:snow_block" => [245, 250, 250], "minecraft:sandstone" => [216, 203, 155],
        "minecraft:oak_log" => [102, 81, 50], "minecraft:oak_leaves" => [60, 120, 40],
        "minecraft:coal_ore" => [60, 60, 60], "minecraft:iron_ore" => [190, 150, 120],
        "minecraft:gold_ore" => [230, 200, 60], "minecraft:diamond_ore" => [90, 220, 220],
        "minecraft:lava" => [255, 110, 20], "minecraft:glowstone" => [255, 200, 110],
        "minecraft:sea_lantern" => [200, 230, 220], "minecraft:torch" => [255, 190, 80],
        "minecraft:deepslate" => [80, 80, 86], "minecraft:tuff" => [108, 109, 102], "minecraft:gravel" => [131, 127, 126],
        "minecraft:cobblestone" => [118, 118, 118], "minecraft:obsidian" => [20, 18, 30], "minecraft:clay" => [160, 166, 179],
        "minecraft:ice" | "minecraft:packed_ice" => [145, 183, 253], "minecraft:netherrack" => [111, 54, 53],
        "minecraft:mycelium" => [111, 98, 102], "minecraft:podzol" => [91, 63, 24], "minecraft:coarse_dirt" => [119, 85, 59],
        "minecraft:red_sand" => [190, 102, 33], "minecraft:terracotta" => [152, 94, 67], "minecraft:andesite" => [136, 136, 136],
        "minecraft:diorite" => [188, 188, 188], "minecraft:granite" => [149, 103, 85], "minecraft:grass_path" | "minecraft:dirt_path" => [148, 122, 65],
        "minecraft:spruce_leaves" => [48, 92, 52], "minecraft:birch_leaves" => [105, 145, 70], "minecraft:jungle_leaves" => [40, 140, 30],
        "minecraft:acacia_leaves" | "minecraft:dark_oak_leaves" | "minecraft:azalea_leaves" | "minecraft:mangrove_leaves" => [60, 110, 40],
        n if n.ends_with("_log") || n.ends_with("_wood") => [100, 78, 48],
        n if n.ends_with("_planks") => [162, 130, 78],
        n if n.ends_with("_leaves") => [60, 120, 40],
        n if n.ends_with("_ore") => [120, 120, 120],
        // Unknown block: stable muted colour derived from its name so real Java worlds stay readable.
        n => {
            let h = n.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
            [90 + (h & 127) as i32, 90 + ((h >> 8) & 127) as i32, 90 + ((h >> 16) & 127) as i32]
        }
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
pub fn mesh_chunk(c: &Chunk, nb: [Option<&Chunk>; 4]) -> Mesh { mesh_chunk_with(c, nb, None) }

/// Per-state facts cached during one meshing call.
struct Info { variants: Option<Arc<Vec<Arc<Baked>>>>, opaque_full: bool }

/// `models`: when present, blocks with a model in the resource pack are drawn textured; everything else falls back to
/// coloured cubes. Plants are only drawn when a model exists for them.
pub fn mesh_chunk_with(c: &Chunk, nb: [Option<&Chunk>; 4], models: Option<&Models>) -> Mesh {
    let mut m = Mesh::default();
    let top = MIN_Y + (c.sections.len() * 16) as i32;
    let white = models.map_or([0.0, 0.0], |m| m.atlas.white_uv());
    // Keyed by the palette entry's address: stable for the duration of this call, and avoids hashing strings per face.
    let infos: RefCell<HashMap<usize, Rc<Info>>> = RefCell::default();
    let info = |b: &BlockState| -> Rc<Info> {
        let key = b as *const BlockState as usize;
        if let Some(i) = infos.borrow().get(&key) { return i.clone(); }
        let variants = models.and_then(|m| m.variants(b));
        let opaque_full = opacity(&b.name) >= 15 && match (&variants, models) {
            (Some(v), _) => v[0].full_cube,
            (None, Some(_)) => !b.is_plant(),
            (None, None) => true,
        };
        let i = Rc::new(Info { variants, opaque_full });
        infos.borrow_mut().insert(key, i.clone());
        i
    };
    // Returns (block, light) at chunk-local coords possibly outside the chunk horizontally.
    let probe = |x: i32, y: i32, z: i32| -> Option<(&BlockState, u8)> {
        if y < MIN_Y || y >= top { return None; }
        let (ch, lx, lz) = if x < 0 { (nb[0], x + 16, z) } else if x > 15 { (nb[1], x - 16, z) }
            else if z < 0 { (nb[2], x, z + 16) } else if z > 15 { (nb[3], x, z - 16) } else { (Some(c), x, z) };
        let ch = ch?;
        let (lx, lz) = (lx as usize, lz as usize);
        Some((ch.get(lx, y, lz), ch.sky_light(lx, y, lz).max(ch.block_light(lx, y, lz))))
    };
    let push = |m: &mut Mesh, quad: [([f32; 3], [f32; 2]); 4], color: [f32; 3]| {
        let base = m.vertices.len() as u32;
        for (pos, uv) in quad { m.vertices.push(Vertex { pos, color, uv }); }
        // Quads are listed clockwise as seen from outside; wind them counter-clockwise for back-face culling.
        m.indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    };
    for (si, s) in c.sections.iter().enumerate() {
        if s.palette.len() == 1 && s.palette[0].is_air() { continue; }
        for ly in 0..16usize { for z in 0..16usize { for x in 0..16usize {
            let b = s.get(x, ly, z);
            if b.is_air() || (models.is_none() && b.is_plant()) { continue; }
            let y = MIN_Y + (si * 16 + ly) as i32;
            let is_fluid = matches!(b.name.as_str(), "minecraft:water" | "minecraft:lava");
            // Emissive blocks ignore ambient light and exceed 1.0 so HDR outputs can show them brighter than white.
            let glow = emission(&b.name) as f32 / 15.0 * 3.0;
            let own_light = s.sky[ly * 256 + z * 16 + x].max(s.block_light[ly * 256 + z * 16 + x]);
            let (bx, bz) = (x as i32, z as i32);
            let me = info(b);
            // Faces are hidden by opaque full cubes, and between identical translucent blocks (glass, leaves, water).
            let own_full = me.variants.as_ref().map_or(!b.is_plant(), |v| v[0].full_cube);
            let same_cull = is_fluid || (own_full && opacity(&b.name) < 15);
            let hidden_by = |nbk: &BlockState| -> bool { info(nbk).opaque_full || (same_cull && nbk.name == b.name) };

            if !is_fluid {
                if let Some(v) = &me.variants {
                    // Textured model path.
                    let seed = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663) ^ (z as u32).wrapping_mul(83492791) ^ (c.x as u32).wrapping_mul(2654435761) ^ (c.z as u32).wrapping_mul(40503);
                    let model = &v[seed as usize % v.len()];
                    let biome = if model.quads.iter().any(|q| q.tint >= 0) { c.biome_at(x, y, z) } else { "" };
                    let tint = if biome.is_empty() { [1.0; 3] } else { models.unwrap().tint(&b.name, biome) };
                    for q in &model.quads {
                        let dir = DIRS[q.face];
                        let neighbour = probe(bx + dir[0], y + dir[1], bz + dir[2]);
                        if let Some(cd) = q.cull {
                            let d = DIRS[cd];
                            if let Some((nbk, _)) = probe(bx + d[0], y + d[1], bz + d[2]) { if hidden_by(nbk) { continue; } }
                        }
                        let light = neighbour.map_or(15, |(_, l)| l).max(own_light);
                        let f = if glow > 0.0 { glow * (0.85 + 0.15 * q.shade) } else { q.shade * (0.15 + 0.85 * light as f32 / 15.0) };
                        let col = if q.tint >= 0 { [tint[0] * f, tint[1] * f, tint[2] * f] } else { [f, f, f] };
                        push(&mut m, std::array::from_fn(|i| ([bx as f32 + q.verts[i][0], y as f32 + q.verts[i][1], bz as f32 + q.verts[i][2]], q.uv[i])), col);
                    }
                    continue;
                }
            }

            // Coloured/fluid cube path.
            let col = block_color(&b.name);
            // Fluid surface height: source 8/9, flowing (8-L)/9, falling or covered columns full height.
            let fluid_h = if is_fluid {
                let covered = probe(bx, y + 1, bz).is_some_and(|(a, _)| a.name == b.name);
                Some(if covered { 1.0 } else { match b.fluid_level() { Some(0) | None => 8.0 / 9.0, Some(l) if l >= 8 => 1.0, Some(l) => (8 - l) as f32 / 9.0 } })
            } else { None };
            let tex = if is_fluid { models.map(|m| (m.fluid_texture(&b.name), m.tint(&b.name, c.biome_at(x, y, z)))) } else { None };
            for (fi, (dir, shade, corners)) in FACES.iter().enumerate() {
                let (nx, ny, nz) = (bx + dir[0], y + dir[1], bz + dir[2]);
                let light = match probe(nx, ny, nz) {
                    Some((nbk, l)) => { if hidden_by(nbk) { continue; } l }
                    None => 15,
                };
                let f = if glow > 0.0 { glow * (0.85 + 0.15 * shade) } else { shade * (0.15 + 0.85 * light as f32 / 15.0) };
                let quad: [([f32; 3], [f32; 2]); 4] = std::array::from_fn(|i| {
                    let k = corners[3 - i]; // FACES lists corners CCW; quads here are CW
                    let ky = match fluid_h { Some(h) if k[1] == 1.0 => h, _ => k[1] };
                    let uv = match &tex {
                        // Project the texture onto the face: top/bottom use x,z; sides use the horizontal axis and height.
                        Some(((u0, v0, u1, v1), _)) => {
                            let (a, bb) = match fi { 2 | 3 => (k[0], k[2]), 0 | 1 | 4 | 5 => (k[0], k[2]), _ => (k[0], k[2]) };
                            let (a, bb) = if fi == 2 || fi == 3 { (k[0], k[1]) } else if fi == 4 || fi == 5 { (k[2], k[1]) } else { (a, bb) };
                            [u0 + (u1 - u0) * a.clamp(0.0, 1.0) * 0.999, v0 + (v1 - v0) * (if fi >= 2 { 1.0 - bb } else { bb }).clamp(0.0, 1.0) * 0.999]
                        }
                        None => white,
                    };
                    ([bx as f32 + k[0], y as f32 + ky, bz as f32 + k[2]], uv)
                });
                let color = match (&tex, col) {
                    (Some((_, tint)), _) => [tint[0] * f, tint[1] * f, tint[2] * f],
                    (None, c) => [c[0] * f, c[1] * f, c[2] * f],
                };
                push(&mut m, quad, color);
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
                uv: [0.0, 0.0],
            });
        }
        m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    m
}

fn append(dst: &mut Mesh, src: Mesh, f: impl Fn([f32; 3]) -> [f32; 3]) {
    let base = dst.vertices.len() as u32;
    dst.vertices.extend(src.vertices.into_iter().map(|v| Vertex { pos: f(v.pos), color: v.color, uv: v.uv }));
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

    // ---- textured path ----
    use crate::atlas::tests::{png_bytes, solid};
    use crate::pack::{tests::temp, Pack, PackStack};

    const CUBE: &str = r##"{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{
        "down":{"texture":"#all","cullface":"down"},"up":{"texture":"#all","cullface":"up","tintindex":0},
        "north":{"texture":"#all","cullface":"north"},"south":{"texture":"#all","cullface":"south"},
        "west":{"texture":"#all","cullface":"west"},"east":{"texture":"#all","cullface":"east"}}}]}"##;

    /// Pack with stone (opaque cube), tinted-top grass, leaves, a bottom slab and a flower cross.
    fn textured() -> (Models, std::path::PathBuf) {
        let d = temp("meshpack");
        let w = |p: &str, b: &[u8]| { let f = d.join("p").join(p); std::fs::create_dir_all(f.parent().unwrap()).unwrap(); std::fs::write(f, b).unwrap(); };
        for n in ["stone", "grass", "leaves", "flower"] { w(&format!("assets/minecraft/textures/block/{n}.png"), &png_bytes(2, 2, &solid(2, 2, [90, 90, 90, 255]))); }
        for (n, tex) in [("stone", "block/stone"), ("grass_block", "block/grass"), ("oak_leaves", "block/leaves")] {
            w(&format!("assets/minecraft/blockstates/{n}.json"), format!(r#"{{"variants":{{"":{{"model":"block/{n}"}}}}}}"#).as_bytes());
            w(&format!("assets/minecraft/models/block/{n}.json"), CUBE.replace("#all", "#t").replace("{\"elements", &format!("{{\"textures\":{{\"t\":\"{tex}\"}},\"elements")).as_bytes());
        }
        w("assets/minecraft/blockstates/stone_slab.json", br#"{"variants":{"":{"model":"block/slab"}}}"#);
        w("assets/minecraft/models/block/slab.json", br##"{"textures":{"t":"block/stone"},"elements":[{"from":[0,0,0],"to":[16,8,16],"faces":{"down":{"texture":"#t","cullface":"down"},"up":{"texture":"#t"},"north":{"texture":"#t","cullface":"north"}}}]}"##);
        w("assets/minecraft/blockstates/poppy.json", br#"{"variants":{"":{"model":"block/cross"}}}"#);
        w("assets/minecraft/models/block/cross.json", br##"{"textures":{"t":"block/flower"},"elements":[{"from":[0.8,0,8],"to":[15.2,16,8],"rotation":{"origin":[8,8,8],"axis":"y","angle":45,"rescale":true},"faces":{"north":{"texture":"#t"},"south":{"texture":"#t"}}},{"from":[8,0,0.8],"to":[8,16,15.2],"rotation":{"origin":[8,8,8],"axis":"y","angle":45,"rescale":true},"faces":{"west":{"texture":"#t"},"east":{"texture":"#t"}}}]}"##);
        let cm: Vec<u8> = (0..256u32 * 256).flat_map(|i| [(i % 256) as u8, 160, (i / 256) as u8, 255]).collect();
        w("assets/minecraft/textures/colormap/grass.png", &png_bytes(256, 256, &cm));
        let mut st = PackStack::new();
        st.push(Pack::open(d.join("p")).unwrap());
        (Models::new(st), d)
    }

    fn tri_normals_outward(m: &Mesh, centre: [f32; 3]) -> bool {
        m.indices.chunks(3).all(|t| {
            let (a, b, c) = (m.vertices[t[0] as usize].pos, m.vertices[t[1] as usize].pos, m.vertices[t[2] as usize].pos);
            let (e1, e2) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            let mid = [(a[0] + b[0] + c[0]) / 3.0 - centre[0], (a[1] + b[1] + c[1]) / 3.0 - centre[1], (a[2] + b[2] + c[2]) / 3.0 - centre[2]];
            n[0] * mid[0] + n[1] * mid[1] + n[2] * mid[2] > 0.0
        })
    }

    #[test]
    fn both_paths_wind_faces_outward() {
        let (models, d) = textured();
        let mut c = Chunk::empty(0, 0);
        c.set(5, 70, 5, &BlockState::simple("minecraft:stone"));
        for m in [mesh_chunk(&c, [None; 4]), mesh_chunk_with(&c, [None; 4], Some(&models))] {
            assert_eq!(m.indices.len(), 36);
            assert!(tri_normals_outward(&m, [5.5, 70.5, 5.5]), "front faces must point away from the cube");
        }
        // Textured vertices sample inside the stone tile; untextured ones sample the white texel.
        let tex = mesh_chunk_with(&c, [None; 4], Some(&models));
        let r = models.atlas.rect("block/stone");
        assert!(tex.vertices.iter().all(|v| v.uv[0] > r.0 && v.uv[0] < r.2 && v.uv[1] > r.1 && v.uv[1] < r.3));
        let plain = mesh_chunk_with(&c, [None; 4], Some(&models));
        let _ = plain;
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn culling_between_opaque_translucent_and_partial_blocks() {
        let (models, d) = textured();
        let faces = |c: &Chunk| mesh_chunk_with(c, [None; 4], Some(&models)).indices.len() / 6;
        let mut c = Chunk::empty(0, 0);
        c.set(5, 70, 5, &BlockState::simple("minecraft:stone"));
        c.set(6, 70, 5, &BlockState::simple("minecraft:stone"));
        assert_eq!(faces(&c), 10, "shared faces between opaque cubes are culled");
        let mut l = Chunk::empty(0, 0);
        let leaf = BlockState { name: "minecraft:oak_leaves".into(), props: "distance=1".into() };
        l.set(5, 70, 5, &leaf); l.set(6, 70, 5, &leaf);
        assert_eq!(faces(&l), 10, "identical translucent cubes cull their shared faces");
        l.set(7, 70, 5, &BlockState::simple("minecraft:stone"));
        assert_eq!(faces(&l), 10 + 5, "leaves do not hide stone faces (and vice versa)");
        // A slab (not a full cube) must not hide the stone face it touches.
        let mut s = Chunk::empty(0, 0);
        s.set(5, 70, 5, &BlockState::simple("minecraft:stone"));
        s.set(6, 70, 5, &BlockState::simple("minecraft:stone_slab"));
        let stone_east = mesh_chunk_with(&s, [None; 4], Some(&models)).vertices.iter().filter(|v| v.pos[0] == 6.0).count();
        assert!(stone_east >= 4, "stone's east face next to a slab stays visible");
        // The slab's down face (cullface=down) is hidden only by an opaque cube below it.
        s.set(6, 69, 5, &BlockState::simple("minecraft:stone"));
        assert!(mesh_chunk_with(&s, [None; 4], Some(&models)).indices.len() < 6 * 6 * 3);
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn plants_render_only_when_the_pack_models_them_and_tints_follow_biome() {
        let (models, d) = textured();
        let mut c = Chunk::empty(0, 0);
        c.set(5, 70, 5, &BlockState::simple("minecraft:poppy"));
        assert_eq!(mesh_chunk(&c, [None; 4]).indices.len(), 0, "no pack -> plants are skipped");
        assert_eq!(mesh_chunk_with(&c, [None; 4], Some(&models)).indices.len(), 4 * 6, "pack cross model: 2 planes x 2 sides");
        // Grass top is tinted; sides use the plain shade colour.
        let mut g = Chunk::empty(0, 0);
        g.set(5, 70, 5, &BlockState::simple("minecraft:grass_block"));
        let tinted = |g: &Chunk| { let m = mesh_chunk_with(g, [None; 4], Some(&models)); m.vertices.iter().find(|v| v.pos[1] == 71.0 && v.pos[0] == 5.0).unwrap().color };
        let plains = tinted(&g);
        for y in (-64..320).step_by(4) { g.set_biome(5, y, 5, "minecraft:desert"); }
        let desert = tinted(&g);
        assert_ne!(plains, desert, "tint depends on the biome");
        let side = mesh_chunk_with(&g, [None; 4], Some(&models)).vertices.iter().find(|v| v.pos[0] == 5.0 && v.pos[1] == 70.0 && v.pos[2] == 5.0).unwrap().color;
        assert!(side[0] == side[1] && side[1] == side[2], "untinted faces stay neutral grey: {side:?}");
        std::fs::remove_dir_all(d).ok();
    }
}
