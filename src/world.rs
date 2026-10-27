//! Java-edition world folder: level.dat + region/r.X.Z.mca.
use crate::anvil::{write_region, Region};
use crate::chunk::Chunk;
use crate::nbt::{self, Tag};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub struct World {
    pub dir: PathBuf,
    pub chunks: HashMap<(i32, i32), Chunk>,
    regions: HashMap<(i32, i32), Option<Region>>,
}

fn region_of(c: i32) -> (i32, usize) { (c >> 5, (c & 31) as usize) }

impl World {
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), chunks: HashMap::new(), regions: HashMap::new() }
    }
    fn region_path(&self, rx: i32, rz: i32) -> PathBuf { self.dir.join("region").join(format!("r.{rx}.{rz}.mca")) }

    /// Loads (and caches) a chunk from disk. Ok(false) if it was never generated.
    pub fn load_chunk(&mut self, cx: i32, cz: i32) -> io::Result<bool> {
        if self.chunks.contains_key(&(cx, cz)) { return Ok(true); }
        let ((rx, lx), (rz, lz)) = (region_of(cx), region_of(cz));
        if !self.regions.contains_key(&(rx, rz)) {
            let p = self.region_path(rx, rz);
            self.regions.insert((rx, rz), if p.exists() { Some(Region::open(p)?) } else { None });
        }
        let Some(Some(r)) = self.regions.get(&(rx, rz)) else { return Ok(false) };
        match r.read_chunk(lx, lz)? {
            Some(t) => match Chunk::from_nbt(&t) {
                Some(c) => { self.chunks.insert((cx, cz), c); Ok(true) }
                None => Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported chunk format (needs 1.18+)")),
            },
            None => Ok(false),
        }
    }

    pub fn insert(&mut self, c: Chunk) { self.chunks.insert((c.x, c.z), c); }

    /// Writes in-memory chunks, preserving untouched chunks already on disk.
    pub fn save(&self) -> io::Result<()> {
        std::fs::create_dir_all(self.dir.join("region"))?;
        let mut by_region: HashMap<(i32, i32), Vec<&Chunk>> = HashMap::new();
        for c in self.chunks.values() { by_region.entry((c.x >> 5, c.z >> 5)).or_default().push(c); }
        for ((rx, rz), cs) in by_region {
            let path = self.region_path(rx, rz);
            let mut out: BTreeMap<(usize, usize), Tag> = BTreeMap::new();
            if path.exists() {
                for (lx, lz, t) in Region::open(&path)?.chunks() { out.insert((lx, lz), t); }
            }
            for c in cs { out.insert(((c.x & 31) as usize, (c.z & 31) as usize), c.to_nbt()); }
            let list: Vec<_> = out.into_iter().map(|((x, z), t)| (x, z, t)).collect();
            std::fs::write(path, write_region(&list)?)?;
        }
        Ok(())
    }
}

pub fn read_level_dat(dir: &Path) -> io::Result<Tag> {
    let mut raw = Vec::new();
    GzDecoder::new(std::fs::File::open(dir.join("level.dat"))?).read_to_end(&mut raw)?;
    Ok(nbt::read_root(&mut &raw[..])?.1)
}

/// World seed from level.dat (handles 1.16+ WorldGenSettings and older RandomSeed).
pub fn level_seed(level: &Tag) -> Option<i64> {
    let d = level.get("Data")?;
    d.get("WorldGenSettings").and_then(|w| w.get("seed")).or_else(|| d.get("RandomSeed"))?.as_i64()
}

pub fn write_level_dat(dir: &Path, name: &str, seed: i64) -> io::Result<()> {
    let wgs = Tag::Compound([("seed".to_string(), Tag::Long(seed))].into());
    let data = Tag::Compound([
        ("LevelName".to_string(), Tag::String(name.into())),
        ("DataVersion".to_string(), Tag::Int(4189)),
        ("WorldGenSettings".to_string(), wgs),
    ].into());
    let root = Tag::Compound([("Data".to_string(), data)].into());
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    nbt::write_root(&mut enc, "", &root)?;
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("level.dat"), enc.finish()?)
}

#[allow(dead_code)]
fn _assert_write(_: &mut dyn Write) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::Generator;
    #[test]
    fn save_and_reload() {
        let dir = std::env::temp_dir().join(format!("be_world_{}", std::process::id()));
        let g = Generator::new(77);
        let mut w = World::open(&dir);
        for (x, z) in [(0, 0), (-1, -1), (33, 2)] { w.insert(g.generate(x, z)); }
        write_level_dat(&dir, "t", 77).unwrap();
        w.save().unwrap();
        assert_eq!(level_seed(&read_level_dat(&dir).unwrap()), Some(77));
        let mut w2 = World::open(&dir);
        assert!(w2.load_chunk(-1, -1).unwrap() && w2.load_chunk(33, 2).unwrap());
        assert!(!w2.load_chunk(5, 5).unwrap());
        let (a, b) = (&w.chunks[&(33, 2)], &w2.chunks[&(33, 2)]);
        for y in [-64, 40, 70, 90] { assert_eq!(a.get(4, y, 4), b.get(4, y, 4)); }
        std::fs::remove_dir_all(dir).ok();
    }
}
