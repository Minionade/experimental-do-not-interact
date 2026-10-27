//! Chunk storage and Java-edition (1.18+) chunk NBT decoding.
use crate::nbt::Tag;

pub const MIN_Y: i32 = -64;
pub const HEIGHT: usize = 384;
pub const SECTIONS: usize = HEIGHT / 16;

/// Block state as a namespaced name; properties kept sorted as "k=v,k=v".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlockState { pub name: String, pub props: String }

impl BlockState {
    pub fn air() -> Self { Self { name: "minecraft:air".into(), props: String::new() } }
    pub fn simple(name: &str) -> Self { Self { name: name.into(), props: String::new() } }
    pub fn is_air(&self) -> bool { matches!(self.name.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air") }
    fn from_nbt(t: &Tag) -> Self {
        let name = t.get("Name").and_then(Tag::as_str).unwrap_or("minecraft:air").to_string();
        let props = match t.get("Properties") {
            Some(Tag::Compound(m)) => m.iter().filter_map(|(k, v)| v.as_str().map(|v| format!("{k}={v}")))
                .collect::<Vec<_>>().join(","),
            _ => String::new(),
        };
        Self { name, props }
    }
}

/// 16^3 section: palette + per-voxel palette index (index = y*256 + z*16 + x).
#[derive(Clone)]
pub struct Section { pub palette: Vec<BlockState>, pub indices: Vec<u16> }

impl Section {
    pub fn empty() -> Self { Self { palette: vec![BlockState::air()], indices: vec![0; 4096] } }
    pub fn get(&self, x: usize, y: usize, z: usize) -> &BlockState {
        &self.palette[self.indices[y * 256 + z * 16 + x] as usize]
    }
    pub fn set(&mut self, x: usize, y: usize, z: usize, b: &BlockState) {
        let id = match self.palette.iter().position(|p| p == b) {
            Some(i) => i,
            None => { self.palette.push(b.clone()); self.palette.len() - 1 }
        };
        self.indices[y * 256 + z * 16 + x] = id as u16;
    }
}

/// Unpacks `count` values of `bits` each from longs; since 1.16 values never straddle longs.
pub fn unpack(longs: &[i64], bits: usize, count: usize) -> Vec<u16> {
    let per = 64 / bits;
    let mask = (1u64 << bits) - 1;
    (0..count).map(|i| {
        let w = longs.get(i / per).copied().unwrap_or(0) as u64;
        ((w >> ((i % per) * bits)) & mask) as u16
    }).collect()
}

pub fn pack(vals: &[u16], bits: usize) -> Vec<i64> {
    let per = 64 / bits;
    let mut out = vec![0i64; vals.len().div_ceil(per)];
    for (i, &v) in vals.iter().enumerate() { out[i / per] |= (v as i64) << ((i % per) * bits); }
    out
}

pub struct Chunk { pub x: i32, pub z: i32, pub sections: Vec<Section> }

impl Chunk {
    pub fn empty(x: i32, z: i32) -> Self { Self { x, z, sections: vec![Section::empty(); SECTIONS] } }

    pub fn get(&self, x: usize, y: i32, z: usize) -> &BlockState {
        let ry = (y - MIN_Y) as usize;
        self.sections[ry / 16].get(x, ry % 16, z)
    }
    pub fn set(&mut self, x: usize, y: i32, z: usize, b: &BlockState) {
        let ry = (y - MIN_Y) as usize;
        self.sections[ry / 16].set(x, ry % 16, z, b)
    }

    /// Decodes a 1.18+ chunk root tag (top-level `sections` list).
    pub fn from_nbt(root: &Tag) -> Option<Self> {
        let x = root.get("xPos")?.as_i64()? as i32;
        let z = root.get("zPos")?.as_i64()? as i32;
        let mut c = Chunk::empty(x, z);
        for s in root.get("sections")?.as_list()? {
            let sy = s.get("Y")?.as_i64()? as i32;
            let idx = sy - MIN_Y / 16;
            if !(0..SECTIONS as i32).contains(&idx) { continue; }
            let Some(bs) = s.get("block_states") else { continue };
            let palette: Vec<BlockState> = bs.get("palette")?.as_list()?.iter().map(BlockState::from_nbt).collect();
            let indices = match bs.get("data").and_then(Tag::as_longs) {
                Some(d) => {
                    let bits = (usize::BITS - (palette.len().max(2) - 1).leading_zeros()).max(4) as usize;
                    unpack(d, bits, 4096)
                }
                None => vec![0; 4096],
            };
            c.sections[idx as usize] = Section { palette, indices };
        }
        Some(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pack_roundtrip() {
        for bits in [4, 5, 7, 9] {
            let v: Vec<u16> = (0..4096).map(|i| (i * 31 % (1 << bits)) as u16).collect();
            assert_eq!(unpack(&pack(&v, bits), bits, 4096), v);
        }
    }
}
