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
pub struct Section {
    pub palette: Vec<BlockState>,
    pub indices: Vec<u16>,
    /// 4x4x4 biome cells, index = y*16 + z*4 + x.
    pub biome_palette: Vec<String>,
    pub biomes: Vec<u8>,
    /// One light level (0..=15) per voxel, same indexing as `indices`.
    pub sky: Vec<u8>,
    pub block_light: Vec<u8>,
}

impl Section {
    pub fn empty() -> Self {
        Self { palette: vec![BlockState::air()], indices: vec![0; 4096],
               biome_palette: vec!["minecraft:plains".into()], biomes: vec![0; 64],
               sky: vec![0; 4096], block_light: vec![0; 4096] }
    }
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

fn unpack_nibbles(b: &[i8]) -> Vec<u8> {
    if b.len() != 2048 { return vec![0; 4096]; }
    (0..4096).map(|i| ((b[i / 2] as u8) >> ((i & 1) * 4)) & 15).collect()
}

fn pack_nibbles(v: &[u8]) -> Vec<i8> {
    v.chunks(2).map(|p| (p[0] | (p[1] << 4)) as i8).collect()
}

fn bits_for(palette_len: usize, min: usize) -> usize {
    ((usize::BITS - (palette_len.max(2) - 1).leading_zeros()) as usize).max(min)
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

    pub fn set_biome(&mut self, x: usize, y: i32, z: usize, biome: &str) {
        let ry = (y - MIN_Y) as usize;
        let sec = &mut self.sections[ry / 16];
        let id = sec.biome_palette.iter().position(|p| p == biome).unwrap_or_else(|| {
            sec.biome_palette.push(biome.to_string()); sec.biome_palette.len() - 1 });
        sec.biomes[((ry % 16) / 4) * 16 + (z / 4) * 4 + x / 4] = id as u8;
    }
    pub fn biome_at(&self, x: usize, y: i32, z: usize) -> &str {
        let ry = (y - MIN_Y) as usize;
        let sec = &self.sections[ry / 16];
        &sec.biome_palette[sec.biomes[((ry % 16) / 4) * 16 + (z / 4) * 4 + x / 4] as usize]
    }
    pub fn sky_light(&self, x: usize, y: i32, z: usize) -> u8 {
        let ry = (y - MIN_Y) as usize;
        self.sections[ry / 16].sky[(ry % 16) * 256 + z * 16 + x]
    }
    pub fn block_light(&self, x: usize, y: i32, z: usize) -> u8 {
        let ry = (y - MIN_Y) as usize;
        self.sections[ry / 16].block_light[(ry % 16) * 256 + z * 16 + x]
    }

    /// Encodes as a 1.18+ chunk root tag.
    pub fn to_nbt(&self) -> Tag {
        use std::collections::BTreeMap;
        let comp = |v: Vec<(&str, Tag)>| Tag::Compound(v.into_iter().map(|(k, t)| (k.to_string(), t)).collect::<BTreeMap<_, _>>());
        let mut secs = Vec::new();
        for (i, s) in self.sections.iter().enumerate() {
            let pal = Tag::List(s.palette.iter().map(|b| {
                let mut v = vec![("Name", Tag::String(b.name.clone()))];
                if !b.props.is_empty() {
                    let m = b.props.split(',').filter_map(|kv| kv.split_once('='))
                        .map(|(k, v)| (k.to_string(), Tag::String(v.to_string()))).collect();
                    v.push(("Properties", Tag::Compound(m)));
                }
                comp(v)
            }).collect());
            let mut bs = vec![("palette", pal)];
            if s.palette.len() > 1 { bs.push(("data", Tag::LongArray(pack(&s.indices, bits_for(s.palette.len(), 4))))); }
            let mut bi = vec![("palette", Tag::List(s.biome_palette.iter().map(|b| Tag::String(b.clone())).collect()))];
            if s.biome_palette.len() > 1 {
                let v: Vec<u16> = s.biomes.iter().map(|&b| b as u16).collect();
                bi.push(("data", Tag::LongArray(pack(&v, bits_for(s.biome_palette.len(), 1)))));
            }
            secs.push(comp(vec![
                ("Y", Tag::Byte((i as i32 + MIN_Y / 16) as i8)),
                ("block_states", comp(bs)), ("biomes", comp(bi)),
                ("SkyLight", Tag::ByteArray(pack_nibbles(&s.sky))),
                ("BlockLight", Tag::ByteArray(pack_nibbles(&s.block_light))),
            ]));
        }
        comp(vec![("DataVersion", Tag::Int(4189)), ("xPos", Tag::Int(self.x)), ("zPos", Tag::Int(self.z)),
            ("yPos", Tag::Int(MIN_Y / 16)), ("Status", Tag::String("minecraft:full".into())),
            ("sections", Tag::List(secs))])
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
            let mut sec = Section::empty();
            sec.palette = palette;
            sec.indices = indices;
            if let Some(b) = s.get("biomes") {
                if let Some(p) = b.get("palette").and_then(Tag::as_list) {
                    sec.biome_palette = p.iter().filter_map(|t| t.as_str().map(String::from)).collect();
                    if sec.biome_palette.is_empty() { sec.biome_palette.push("minecraft:plains".into()); }
                    if let Some(d) = b.get("data").and_then(Tag::as_longs) {
                        let bits = (usize::BITS - (sec.biome_palette.len().max(2) - 1).leading_zeros()).max(1) as usize;
                        sec.biomes = unpack(d, bits, 64).into_iter().map(|v| v as u8).collect();
                    }
                }
            }
            if let Some(Tag::ByteArray(l)) = s.get("SkyLight") { sec.sky = unpack_nibbles(l); }
            if let Some(Tag::ByteArray(l)) = s.get("BlockLight") { sec.block_light = unpack_nibbles(l); }
            c.sections[idx as usize] = sec;
        }
        Some(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunk_nbt_roundtrip() {
        let mut c = Chunk::empty(-2, 5);
        c.set(1, 70, 2, &BlockState::simple("minecraft:stone"));
        c.set(15, -64, 15, &BlockState { name: "minecraft:oak_log".into(), props: "axis=y".into() });
        c.set_biome(9, 70, 9, "minecraft:desert");
        let d = Chunk::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!((d.x, d.z), (-2, 5));
        assert_eq!(d.get(1, 70, 2).name, "minecraft:stone");
        assert_eq!(d.get(15, -64, 15).props, "axis=y");
        assert_eq!(d.biome_at(9, 70, 9), "minecraft:desert");
        assert!(d.get(0, 0, 0).is_air());
    }
    #[test]
    fn pack_roundtrip() {
        for bits in [4, 5, 7, 9] {
            let v: Vec<u16> = (0..4096).map(|i| (i * 31 % (1 << bits)) as u16).collect();
            assert_eq!(unpack(&pack(&v, bits), bits, 4096), v);
        }
    }
}
