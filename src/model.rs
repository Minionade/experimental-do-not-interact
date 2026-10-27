//! Block models from resource packs: blockstate JSON -> model JSON -> baked quads in atlas UV space.
use crate::atlas::{decode_png, Atlas};
use crate::chunk::BlockState;
use crate::pack::{split_id, PackStack};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// down, up, north, south, west, east
pub const DIRS: [[i32; 3]; 6] = [[0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]];
const DIR_NAMES: [&str; 6] = ["down", "up", "north", "south", "west", "east"];
const FACE_SHADE: [f32; 6] = [0.5, 1.0, 0.8, 0.8, 0.6, 0.6];

#[derive(Clone, Debug)]
pub struct Quad {
    /// Corners in block-local space (0..1), ordered top-left, top-right, bottom-right, bottom-left of the texture.
    pub verts: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    /// Which neighbour hides this quad when it is an opaque cube (rotated with the model).
    pub cull: Option<usize>,
    /// Direction the quad faces (dominant axis), used for shading and for sampling light.
    pub face: usize,
    pub tint: i32,
    pub shade: f32,
}

#[derive(Debug, Default)]
pub struct Baked { pub quads: Vec<Quad>, pub full_cube: bool }

struct Element { from: [f32; 3], to: [f32; 3], rot: Option<ElemRot>, shade: bool, faces: [Option<FaceDef>; 6] }
struct ElemRot { origin: [f32; 3], axis: usize, angle: f32, rescale: bool }
struct FaceDef { uv: Option<[f32; 4]>, texture: String, cull: Option<usize>, rotation: i32, tint: i32 }

struct Resolved { textures: HashMap<String, String>, elements: Vec<Element> }

pub struct Models {
    pub atlas: Atlas,
    packs: PackStack,
    resolved: Mutex<HashMap<String, Option<Arc<Resolved>>>>,
    states: RwLock<HashMap<(String, String), Option<Arc<Vec<Arc<Baked>>>>>>,
    grass: Option<Vec<u8>>,
    foliage: Option<Vec<u8>>,
}

fn f3(v: &Value) -> Option<[f32; 3]> { let a = v.as_array()?; Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32]) }

fn dir_index(name: &str) -> Option<usize> { DIR_NAMES.iter().position(|d| *d == name) }

fn axis_index(c: &str) -> usize { match c { "x" => 0, "y" => 1, _ => 2 } }

fn nearest_dir(v: [f32; 3]) -> usize {
    let a = [v[0].abs(), v[1].abs(), v[2].abs()];
    let k = if a[0] >= a[1] && a[0] >= a[2] { 0 } else if a[1] >= a[2] { 1 } else { 2 };
    match (k, v[k] >= 0.0) { (0, false) => 4, (0, true) => 5, (1, false) => 0, (1, true) => 1, (_, false) => 2, (_, true) => 3 }
}

/// Block-local corner positions (0..1) of face `d`, in texture order TL, TR, BR, BL (see module docs).
fn face_corners(d: usize, f: [f32; 3], t: [f32; 3]) -> [[f32; 3]; 4] {
    let ([x0, y0, z0], [x1, y1, z1]) = (f, t);
    match d {
        0 => [[x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0]],
        1 => [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
        2 => [[x1, y1, z0], [x0, y1, z0], [x0, y0, z0], [x1, y0, z0]],
        3 => [[x0, y1, z1], [x1, y1, z1], [x1, y0, z1], [x0, y0, z1]],
        4 => [[x0, y1, z0], [x0, y1, z1], [x0, y0, z1], [x0, y0, z0]],
        _ => [[x1, y1, z1], [x1, y1, z0], [x1, y0, z0], [x1, y0, z1]],
    }
}

/// Default UVs (0..16) derived from the element box, per the documented model format.
fn default_uv(d: usize, f: [f32; 3], t: [f32; 3]) -> [f32; 4] {
    let ([x0, y0, z0], [x1, y1, z1]) = (f, t);
    match d {
        0 => [x0, 16.0 - z1, x1, 16.0 - z0],
        1 => [x0, z0, x1, z1],
        2 => [16.0 - x1, 16.0 - y1, 16.0 - x0, 16.0 - y0],
        3 => [x0, 16.0 - y1, x1, 16.0 - y0],
        4 => [z0, 16.0 - y1, z1, 16.0 - y0],
        _ => [16.0 - z1, 16.0 - y1, 16.0 - z0, 16.0 - y0],
    }
}

/// Rotates `p` about `o` around axis `a` (0=x,1=y,2=z) by `deg` degrees (right-handed).
fn rot_axis(p: [f32; 3], o: [f32; 3], a: usize, deg: f32) -> [f32; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    let q = [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
    let r = match a {
        0 => [q[0], q[1] * c - q[2] * s, q[1] * s + q[2] * c],
        1 => [q[0] * c + q[2] * s, q[1], -q[0] * s + q[2] * c],
        _ => [q[0] * c - q[1] * s, q[0] * s + q[1] * c, q[2]],
    };
    [r[0] + o[0], r[1] + o[1], r[2] + o[2]]
}

/// Blockstate x/y rotation: x:A tilts north toward up for A=270 and toward down for A=90; y:A turns clockwise seen from above.
fn model_rot(p: [f32; 3], xr: i32, yr: i32) -> [f32; 3] {
    let o = [0.5, 0.5, 0.5];
    let p = if xr != 0 { rot_axis(p, o, 0, -(xr as f32)) } else { p };
    if yr != 0 { rot_axis(p, o, 1, -(yr as f32)) } else { p }
}

const BIOMES: &[(&str, f32, f32, u32)] = &[
    ("plains", 0.8, 0.4, 0x3F76E4), ("sunflower_plains", 0.8, 0.4, 0x3F76E4), ("forest", 0.7, 0.8, 0x3F76E4), ("flower_forest", 0.7, 0.8, 0x3F76E4),
    ("birch_forest", 0.6, 0.6, 0x3F76E4), ("dark_forest", 0.7, 0.8, 0x3F76E4), ("taiga", 0.25, 0.8, 0x3F76E4), ("snowy_taiga", -0.5, 0.4, 0x3D57D6),
    ("old_growth_pine_taiga", 0.3, 0.8, 0x3F76E4), ("snowy_plains", 0.0, 0.5, 0x3D57D6), ("ice_spikes", 0.0, 0.5, 0x3D57D6),
    ("desert", 2.0, 0.0, 0x3F76E4), ("savanna", 1.2, 0.0, 0x3F76E4), ("savanna_plateau", 1.0, 0.0, 0x3F76E4), ("badlands", 2.0, 0.0, 0x3F76E4),
    ("jungle", 0.95, 0.9, 0x3F76E4), ("sparse_jungle", 0.95, 0.8, 0x3F76E4), ("bamboo_jungle", 0.95, 0.9, 0x3F76E4),
    ("swamp", 0.8, 0.9, 0x617B64), ("mangrove_swamp", 0.8, 0.9, 0x3A7A6A), ("river", 0.5, 0.5, 0x3F76E4), ("frozen_river", 0.0, 0.5, 0x3938C9),
    ("beach", 0.8, 0.4, 0x3F76E4), ("snowy_beach", 0.05, 0.3, 0x3D57D6), ("stony_shore", 0.2, 0.3, 0x3F76E4),
    ("ocean", 0.5, 0.5, 0x3F76E4), ("deep_ocean", 0.5, 0.5, 0x3F76E4), ("lukewarm_ocean", 0.5, 0.5, 0x45ADF2), ("warm_ocean", 0.5, 0.5, 0x43D5EE),
    ("cold_ocean", 0.5, 0.5, 0x3D57D6), ("frozen_ocean", 0.0, 0.5, 0x3938C9), ("deep_frozen_ocean", 0.5, 0.5, 0x3938C9),
    ("windswept_hills", 0.2, 0.3, 0x3F76E4), ("windswept_forest", 0.2, 0.3, 0x3F76E4), ("windswept_gravelly_hills", 0.2, 0.3, 0x3F76E4),
    ("meadow", 0.5, 0.8, 0x0E4ECF), ("grove", -0.2, 0.8, 0x3D57D6), ("snowy_slopes", -0.3, 0.9, 0x3D57D6), ("jagged_peaks", -0.7, 0.9, 0x3D57D6),
    ("frozen_peaks", -0.7, 0.9, 0x3D57D6), ("stony_peaks", 1.0, 0.3, 0x3F76E4), ("cherry_grove", 0.5, 0.8, 0x5DB7EF), ("mushroom_fields", 0.9, 1.0, 0x3F76E4),
];

/// (temperature, downfall, water colour) for a biome id; unknown biomes behave like plains.
pub fn biome_params(id: &str) -> (f32, f32, u32) {
    let n = id.strip_prefix("minecraft:").unwrap_or(id);
    BIOMES.iter().find(|b| b.0 == n).map_or((0.8, 0.4, 0x3F76E4), |b| (b.1, b.2, b.3))
}

fn rgb(c: u32) -> [f32; 3] { [((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0] }

impl Models {
    pub fn new(packs: PackStack) -> Models {
        let atlas = Atlas::build(&packs);
        let cmap = |n: &str| packs.read(&format!("assets/minecraft/textures/colormap/{n}.png")).and_then(|b| decode_png(&b))
            .filter(|i| i.w == 256 && i.h == 256).map(|i| i.rgba);
        let (grass, foliage) = (cmap("grass"), cmap("foliage"));
        Models { atlas, packs, resolved: Mutex::default(), states: RwLock::default(), grass, foliage }
    }

    fn colormap(map: &Option<Vec<u8>>, biome: &str, default: u32) -> [f32; 3] {
        let (t, d, _) = biome_params(biome);
        let t = t.clamp(0.0, 1.0);
        let r = d.clamp(0.0, 1.0) * t;
        match map {
            Some(px) => { let (x, y) = (((1.0 - t) * 255.0) as usize, ((1.0 - r) * 255.0) as usize); let i = (y * 256 + x) * 4;
                [px[i] as f32 / 255.0, px[i + 1] as f32 / 255.0, px[i + 2] as f32 / 255.0] }
            None => rgb(default),
        }
    }

    /// Colour multiplier for a tinted quad (tintindex >= 0) of `block` in `biome`.
    pub fn tint(&self, block: &str, biome: &str) -> [f32; 3] {
        let n = block.strip_prefix("minecraft:").unwrap_or(block);
        match n {
            "spruce_leaves" => rgb(0x619961),
            "birch_leaves" => rgb(0x80A755),
            "lily_pad" => rgb(0x208030),
            "water" | "water_cauldron" => rgb(biome_params(biome).2),
            "vine" => Self::colormap(&self.foliage, biome, 0x77AB2F),
            _ if n.ends_with("_leaves") => Self::colormap(&self.foliage, biome, 0x77AB2F),
            _ => Self::colormap(&self.grass, biome, 0x91BD59),
        }
    }

    /// Fluid texture rect and whether it is tinted.
    pub fn fluid_texture(&self, name: &str) -> (f32, f32, f32, f32) {
        self.atlas.rect(if name == "minecraft:lava" { "block/lava_still" } else { "block/water_still" })
    }

    fn resolve(&self, id: &str) -> Option<Arc<Resolved>> {
        if let Some(r) = self.resolved.lock().unwrap().get(id) { return r.clone(); }
        let r = self.resolve_inner(id, 0).map(Arc::new);
        self.resolved.lock().unwrap().insert(id.to_string(), r.clone());
        r
    }

    fn resolve_inner(&self, id: &str, depth: u32) -> Option<Resolved> {
        if depth > 16 { return None; }
        let (ns, path) = split_id(id);
        let json = self.packs.read_json(&format!("assets/{ns}/models/{path}.json"))?;
        let mut out = match json.get("parent").and_then(|p| p.as_str()) {
            Some(p) if !p.starts_with("builtin/") => self.resolve_inner(p, depth + 1).unwrap_or(Resolved { textures: HashMap::new(), elements: vec![] }),
            _ => Resolved { textures: HashMap::new(), elements: vec![] },
        };
        if let Some(t) = json.get("textures").and_then(|t| t.as_object()) {
            for (k, v) in t { if let Some(s) = v.as_str() { out.textures.insert(k.clone(), s.to_string()); } }
        }
        if let Some(els) = json.get("elements").and_then(|e| e.as_array()) {
            out.elements = els.iter().filter_map(parse_element).collect();
        }
        Some(out)
    }

    fn texture_of(&self, r: &Resolved, t: &str) -> String {
        let mut cur = t.to_string();
        for _ in 0..16 {
            match cur.strip_prefix('#').map(str::to_string) {
                Some(name) => match r.textures.get(&name) { Some(n) => cur = n.clone(), None => return "missing".into() },
                None => return cur,
            }
        }
        "missing".into()
    }

    fn bake_model(&self, model: &str, xr: i32, yr: i32) -> Baked {
        let Some(r) = self.resolve(model) else { return Baked::default() };
        let mut out = Baked::default();
        for el in &r.elements {
            if el.rot.is_none() && el.from == [0.0; 3] && el.to == [16.0; 3] { out.full_cube = true; }
            let (f, t) = (el.from.map(|v| v / 16.0), el.to.map(|v| v / 16.0));
            for (d, face) in el.faces.iter().enumerate() {
                let Some(face) = face else { continue };
                let mut verts = face_corners(d, f, t);
                if let Some(er) = &el.rot {
                    let o = er.origin.map(|v| v / 16.0);
                    for v in &mut verts {
                        *v = rot_axis(*v, o, er.axis, er.angle);
                        if er.rescale {
                            let k = 1.0 / er.angle.to_radians().cos().abs();
                            for a in 0..3 { if a != er.axis { v[a] = o[a] + (v[a] - o[a]) * k; } }
                        }
                    }
                }
                for v in &mut verts { *v = model_rot(*v, xr, yr); }
                let uvr = face.uv.unwrap_or_else(|| default_uv(d, el.from, el.to));
                let corners = [(uvr[0], uvr[1]), (uvr[2], uvr[1]), (uvr[2], uvr[3]), (uvr[0], uvr[3])];
                let shift = (face.rotation.rem_euclid(360) / 90) as usize;
                let rect = self.atlas.rect(&self.texture_of(&r, &face.texture));
                let eps = 0.0001; // keep samples inside the tile
                let mut uv = [[0.0; 2]; 4];
                for i in 0..4 {
                    let (u, v) = corners[(i + 4 - shift) % 4];
                    uv[i] = [rect.0 + (rect.2 - rect.0) * (u / 16.0).clamp(eps, 1.0 - eps), rect.1 + (rect.3 - rect.1) * (v / 16.0).clamp(eps, 1.0 - eps)];
                }
                let e1 = [verts[1][0] - verts[0][0], verts[1][1] - verts[0][1], verts[1][2] - verts[0][2]];
                let e3 = [verts[3][0] - verts[0][0], verts[3][1] - verts[0][1], verts[3][2] - verts[0][2]];
                let n = [-(e1[1] * e3[2] - e1[2] * e3[1]), -(e1[2] * e3[0] - e1[0] * e3[2]), -(e1[0] * e3[1] - e1[1] * e3[0])];
                let facing = nearest_dir(n);
                // Rotate the cull direction with the model (as a vector about the origin, not the block centre).
                let cull = face.cull.map(|c| { let v = DIRS[c]; let p = model_rot([0.5 + v[0] as f32, 0.5 + v[1] as f32, 0.5 + v[2] as f32], xr, yr); nearest_dir([p[0] - 0.5, p[1] - 0.5, p[2] - 0.5]) });
                out.quads.push(Quad { verts, uv, cull, face: facing, tint: face.tint, shade: if el.shade { FACE_SHADE[facing] } else { 1.0 } });
            }
        }
        out
    }

    fn bake_state(&self, name: &str, props: &str) -> Option<Vec<Arc<Baked>>> {
        let (ns, path) = split_id(name);
        let bs = self.packs.read_json(&format!("assets/{ns}/blockstates/{path}.json"))?;
        let pmap: HashMap<&str, &str> = props.split(',').filter_map(|kv| kv.split_once('=')).collect();
        let apply = |v: &Value| -> Vec<Arc<Baked>> {
            let opts: Vec<&Value> = match v { Value::Array(a) => a.iter().collect(), o => vec![o] };
            opts.into_iter().filter_map(|o| {
                let m = o.get("model")?.as_str()?;
                let (x, y) = (o.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32, o.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32);
                Some(Arc::new(self.bake_model(m, x, y)))
            }).collect()
        };
        if let Some(variants) = bs.get("variants").and_then(|v| v.as_object()) {
            let mut best: Option<(usize, &Value)> = None;
            for (k, v) in variants {
                let conds: Vec<(&str, &str)> = k.split(',').filter_map(|kv| kv.split_once('=')).collect();
                if conds.iter().all(|(p, val)| pmap.get(p) == Some(val)) && best.is_none_or(|(n, _)| conds.len() > n) { best = Some((conds.len(), v)); }
            }
            let list = apply(best?.1);
            return if list.is_empty() { None } else { Some(list) };
        }
        let parts = bs.get("multipart")?.as_array()?;
        let mut quads = Baked::default();
        for part in parts {
            if part.get("when").is_none_or(|w| cond_matches(w, &pmap)) {
                // Multipart picks one weighted option per part; use the first.
                if let Some(b) = part.get("apply").map(apply).and_then(|l| l.into_iter().next()) {
                    quads.full_cube |= b.full_cube;
                    quads.quads.extend(b.quads.iter().cloned());
                }
            }
        }
        Some(vec![Arc::new(quads)])
    }

    /// All weighted variants for a block state (cached), if the pack defines a model for it.
    pub fn variants(&self, b: &BlockState) -> Option<Arc<Vec<Arc<Baked>>>> {
        let key = (b.name.clone(), b.props.clone());
        if let Some(c) = self.states.read().unwrap().get(&key) { return c.clone(); }
        let baked = self.bake_state(&b.name, &b.props).map(Arc::new);
        self.states.write().unwrap().insert(key, baked.clone());
        baked
    }

    /// Baked model for a block state. `seed` picks among weighted variants.
    pub fn model(&self, b: &BlockState, seed: u32) -> Option<Arc<Baked>> {
        self.variants(b).map(|l| l[seed as usize % l.len()].clone())
    }
}

fn cond_matches(w: &Value, pmap: &HashMap<&str, &str>) -> bool {
    let Some(o) = w.as_object() else { return false };
    if let Some(any) = o.get("OR").and_then(|v| v.as_array()) { return any.iter().any(|c| cond_matches(c, pmap)); }
    if let Some(all) = o.get("AND").and_then(|v| v.as_array()) { return all.iter().all(|c| cond_matches(c, pmap)); }
    o.iter().all(|(k, v)| {
        let want = match v { Value::String(s) => s.clone(), Value::Bool(b) => b.to_string(), o => o.to_string() };
        want.split('|').any(|alt| pmap.get(k.as_str()) == Some(&alt))
    })
}

fn parse_element(e: &Value) -> Option<Element> {
    let (from, to) = (f3(e.get("from")?)?, f3(e.get("to")?)?);
    let rot = e.get("rotation").and_then(|r| Some(ElemRot {
        origin: f3(r.get("origin")?)?, axis: axis_index(r.get("axis")?.as_str()?), angle: r.get("angle")?.as_f64()? as f32,
        rescale: r.get("rescale").and_then(|v| v.as_bool()).unwrap_or(false) }));
    let mut faces: [Option<FaceDef>; 6] = Default::default();
    for (name, f) in e.get("faces")?.as_object()? {
        let Some(d) = dir_index(name) else { continue };
        let uv = f.get("uv").and_then(|u| u.as_array()).and_then(|u| Some([u.first()?.as_f64()? as f32, u.get(1)?.as_f64()? as f32, u.get(2)?.as_f64()? as f32, u.get(3)?.as_f64()? as f32]));
        faces[d] = Some(FaceDef { uv, texture: f.get("texture")?.as_str()?.to_string(),
            cull: f.get("cullface").and_then(|c| c.as_str()).and_then(dir_index),
            rotation: f.get("rotation").and_then(|r| r.as_i64()).unwrap_or(0) as i32,
            tint: f.get("tintindex").and_then(|t| t.as_i64()).unwrap_or(-1) as i32 });
    }
    Some(Element { from, to, rot, shade: e.get("shade").and_then(|s| s.as_bool()).unwrap_or(true), faces })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::tests::{png_bytes, solid};
    use crate::pack::{tests::temp, Pack};

    const CUBE_ALL: &str = r##"{"parent":"block/cube","textures":{"particle":"#all","down":"#all","up":"#all","north":"#all","east":"#all","south":"#all","west":"#all"}}"##;
    const CUBE: &str = r##"{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{
        "down":{"texture":"#down","cullface":"down"},"up":{"texture":"#up","cullface":"up"},
        "north":{"texture":"#north","cullface":"north"},"south":{"texture":"#south","cullface":"south"},
        "west":{"texture":"#west","cullface":"west"},"east":{"texture":"#east","cullface":"east"}}}]}"##;

    fn pack(files: &[(&str, Vec<u8>)]) -> (Models, std::path::PathBuf) {
        let d = temp("model");
        let base: Vec<(&str, Vec<u8>)> = vec![
            ("assets/minecraft/models/block/cube.json", CUBE.as_bytes().to_vec()),
            ("assets/minecraft/models/block/cube_all.json", CUBE_ALL.as_bytes().to_vec()),
            ("assets/minecraft/textures/block/stone.png", png_bytes(2, 2, &solid(2, 2, [120, 120, 120, 255]))),
            ("assets/minecraft/textures/block/dirt.png", png_bytes(2, 2, &solid(2, 2, [130, 90, 60, 255]))),
        ];
        for (p, b) in base.iter().map(|(p, b)| (*p, b)).chain(files.iter().map(|(p, b)| (*p, b))) {
            let f = d.join("p").join(p); std::fs::create_dir_all(f.parent().unwrap()).unwrap(); std::fs::write(f, b).unwrap();
        }
        let mut s = PackStack::new();
        s.push(Pack::open(d.join("p")).unwrap());
        (Models::new(s), d)
    }

    fn bs(name: &str, props: &str) -> BlockState { BlockState { name: name.into(), props: props.into() } }

    #[test]
    fn cube_all_bakes_six_culled_quads_with_correct_uv_orientation() {
        let (m, d) = pack(&[("assets/minecraft/blockstates/stone.json", br##"{"variants":{"":{"model":"block/stone"}}}"##.to_vec()),
            ("assets/minecraft/models/block/stone.json", br##"{"parent":"block/cube_all","textures":{"all":"block/stone"}}"##.to_vec())]);
        let b = m.model(&bs("minecraft:stone", ""), 0).unwrap();
        assert!(b.full_cube);
        assert_eq!(b.quads.len(), 6);
        assert!(b.quads.iter().all(|q| q.cull == Some(q.face) && q.tint == -1));
        let rect = m.atlas.rect("block/stone");
        for q in &b.quads { for uv in q.uv { assert!(uv[0] > rect.0 && uv[0] < rect.2 && uv[1] > rect.1 && uv[1] < rect.3); } }
        let up = b.quads.iter().find(|q| q.face == 1).unwrap();
        assert_eq!(up.verts[0], [0.0, 1.0, 0.0]);                     // texture top-left = north-west corner
        assert_eq!(up.verts[2], [1.0, 1.0, 1.0]);                     // bottom-right = south-east corner
        assert!(up.uv[0][0] < up.uv[1][0] && up.uv[0][1] < up.uv[3][1], "u grows to the right, v grows downward");
        let north = b.quads.iter().find(|q| q.face == 2).unwrap();
        assert_eq!(north.verts[0], [1.0, 1.0, 0.0], "seen from the north, +x is on the left");
        assert_eq!(up.shade, 1.0);
        assert_eq!(b.quads.iter().find(|q| q.face == 0).unwrap().shade, 0.5);
        assert!(m.model(&bs("minecraft:nothing", ""), 0).is_none());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn blockstate_rotations_move_faces_and_cull_directions() {
        // A model with only a north face, rotated by every blockstate option.
        let (m, d) = pack(&[
            ("assets/minecraft/models/block/front.json", br##"{"textures":{"t":"block/dirt"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#t","cullface":"north"}}}]}"##.to_vec()),
            ("assets/minecraft/blockstates/furnace.json", br##"{"variants":{"facing=north":{"model":"block/front"},"facing=east":{"model":"block/front","y":90},
                "facing=south":{"model":"block/front","y":180},"facing=west":{"model":"block/front","y":270},
                "facing=up":{"model":"block/front","x":270},"facing=down":{"model":"block/front","x":90}}}"##.to_vec()),
        ]);
        let face = |p: &str| { let q = &m.model(&bs("minecraft:furnace", p), 0).unwrap().quads[0]; (q.face, q.cull) };
        assert_eq!(face("facing=north"), (2, Some(2)));
        assert_eq!(face("facing=east"), (5, Some(5)));
        assert_eq!(face("facing=south"), (3, Some(3)));
        assert_eq!(face("facing=west"), (4, Some(4)));
        assert_eq!(face("facing=up"), (1, Some(1)));
        assert_eq!(face("facing=down"), (0, Some(0)));
        // Extra properties not mentioned by the variant keys are ignored.
        assert_eq!(face("facing=east,lit=true"), (5, Some(5)));
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn multipart_conditions_and_or() {
        let (m, d) = pack(&[
            ("assets/minecraft/models/block/post.json", br##"{"textures":{"t":"block/dirt"},"elements":[{"from":[6,0,6],"to":[10,16,10],"faces":{"up":{"texture":"#t"}}}]}"##.to_vec()),
            ("assets/minecraft/models/block/side.json", br##"{"textures":{"t":"block/dirt"},"elements":[{"from":[7,6,0],"to":[9,9,9],"faces":{"up":{"texture":"#t"},"down":{"texture":"#t"}}}]}"##.to_vec()),
            ("assets/minecraft/blockstates/fence.json", br##"{"multipart":[{"apply":{"model":"block/post"}},
                {"when":{"north":"true"},"apply":{"model":"block/side"}},
                {"when":{"OR":[{"east":"true"},{"west":"true"}]},"apply":{"model":"block/side","y":90}},
                {"when":{"south":"true|maybe"},"apply":{"model":"block/side","y":180}}]}"##.to_vec()),
        ]);
        let n = |p: &str| m.model(&bs("minecraft:fence", p), 0).unwrap().quads.len();
        assert_eq!(n("north=false,east=false,south=false,west=false"), 1);
        assert_eq!(n("north=true,east=false,south=false,west=false"), 3);
        assert_eq!(n("north=true,east=true,south=false,west=true"), 5, "OR matches once per part");
        assert_eq!(n("north=false,east=false,south=maybe,west=false"), 3, "alternatives with |");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn element_rotation_rescale_and_uv_rotation() {
        let (m, d) = pack(&[
            ("assets/minecraft/models/block/cross.json", br##"{"textures":{"t":"block/dirt"},"elements":[{"from":[0.8,0,8],"to":[15.2,16,8],"rotation":{"origin":[8,8,8],"axis":"y","angle":45,"rescale":true},"shade":false,"faces":{"north":{"texture":"#t","uv":[0,0,16,16],"tintindex":0},"south":{"texture":"#t"}}}]}"##.to_vec()),
            ("assets/minecraft/blockstates/plant.json", br##"{"variants":{"":{"model":"block/cross"}}}"##.to_vec()),
            ("assets/minecraft/models/block/rot.json", br##"{"textures":{"t":"block/dirt"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#t","uv":[0,0,8,16],"rotation":90}}}]}"##.to_vec()),
            ("assets/minecraft/blockstates/rotated.json", br##"{"variants":{"":{"model":"block/rot"}}}"##.to_vec()),
        ]);
        let b = m.model(&bs("minecraft:plant", ""), 0).unwrap();
        assert!(!b.full_cube);
        let q = &b.quads[0];
        let span = |axis: usize| { let v: Vec<f32> = q.verts.iter().map(|p| p[axis]).collect();
            v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min) };
        // A 14.4-wide plane turned 45 degrees with rescale keeps its 0.9 extent on both horizontal axes (a cross-plane diagonal).
        assert!((span(0) - 0.9).abs() < 1e-3 && (span(2) - 0.0).abs() > 0.5, "x span {} z span {}", span(0), span(2));
        assert_eq!(q.tint, 0);
        assert_eq!(q.shade, 1.0, "shade:false is unshaded");
        // UV rotation: rotating 90 degrees shifts which uv corner lands on which vertex.
        let r = m.model(&bs("minecraft:rotated", ""), 0).unwrap();
        let rect = m.atlas.rect("block/dirt");
        let nuv = |p: [f32; 2]| [((p[0] - rect.0) / (rect.2 - rect.0) * 16.0).round(), ((p[1] - rect.1) / (rect.3 - rect.1) * 16.0).round()];
        // Unrotated TL would be (0,0); rotated by 90 the TL vertex shows the bottom-left of the uv rect (0,16).
        assert_eq!(nuv(r.quads[0].uv[0]), [0.0, 16.0]);
        assert_eq!(nuv(r.quads[0].uv[1]), [0.0, 0.0]);
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn weighted_variants_and_missing_textures() {
        let (m, d) = pack(&[
            ("assets/minecraft/blockstates/dirt.json", br##"{"variants":{"":[{"model":"block/a"},{"model":"block/b","y":90}]}}"##.to_vec()),
            ("assets/minecraft/models/block/a.json", br##"{"parent":"block/cube_all","textures":{"all":"block/dirt"}}"##.to_vec()),
            ("assets/minecraft/models/block/b.json", br##"{"parent":"block/cube_all","textures":{"all":"block/ghost_texture"}}"##.to_vec()),
        ]);
        let a = m.model(&bs("minecraft:dirt", ""), 0).unwrap();
        let b = m.model(&bs("minecraft:dirt", ""), 1).unwrap();
        assert_ne!(a.quads[0].uv[0], b.quads[0].uv[0], "variant 1 uses the missing-texture tile");
        let miss = m.atlas.rect("blockengine:missing");
        assert!(b.quads[0].uv[0][0] >= miss.0 && b.quads[0].uv[0][0] <= miss.2);
        assert!(Arc::ptr_eq(&m.model(&bs("minecraft:dirt", ""), 2).unwrap(), &a), "seed wraps around the variant list");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn biome_tints_use_colormaps_with_fallbacks() {
        // Colormap whose red channel encodes x and green channel encodes y.
        let cm: Vec<u8> = (0..256u32 * 256).flat_map(|i| [(i % 256) as u8, (i / 256) as u8, 0, 255]).collect();
        let (m, d) = pack(&[("assets/minecraft/textures/colormap/grass.png", png_bytes(256, 256, &cm))]);
        let plains = m.tint("minecraft:grass_block", "minecraft:plains");
        let desert = m.tint("minecraft:grass_block", "minecraft:desert");
        let snow = m.tint("minecraft:grass_block", "minecraft:snowy_plains");
        assert!(desert[0] < plains[0] && plains[0] < snow[0], "hotter biomes sample further left (x = (1 - temperature) * 255)");
        assert_eq!(m.tint("minecraft:spruce_leaves", "minecraft:plains"), rgb(0x619961));
        assert_eq!(m.tint("minecraft:water", "minecraft:swamp"), rgb(0x617B64));
        // No foliage colormap in this pack -> built-in default colour.
        assert_eq!(m.tint("minecraft:oak_leaves", "minecraft:plains"), rgb(0x77AB2F));
        assert_eq!(biome_params("minecraft:never_heard_of_it").0, 0.8);
        std::fs::remove_dir_all(d).ok();
    }
}
