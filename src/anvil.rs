//! Anvil region (.mca) files: 32x32 chunks, 4 KiB sectors.
use crate::nbt::{self, Tag};
use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{self, Read, Write};
use std::path::Path;

const SECTOR: usize = 4096;

pub struct Region {
    data: Vec<u8>,
}

impl Region {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self { data: std::fs::read(path)? })
    }
    pub fn from_bytes(data: Vec<u8>) -> Self { Self { data } }

    /// Reads chunk at local coords (0..32). Ok(None) if not generated.
    pub fn read_chunk(&self, lx: usize, lz: usize) -> io::Result<Option<Tag>> {
        let i = (lx & 31) + (lz & 31) * 32;
        if self.data.len() < SECTOR * 2 { return Ok(None); }
        let e = u32::from_be_bytes(self.data[i * 4..i * 4 + 4].try_into().unwrap());
        if e == 0 { return Ok(None); }
        let off = (e >> 8) as usize * SECTOR;
        let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
        let hdr = self.data.get(off..off + 5).ok_or_else(|| bad("chunk offset out of range"))?;
        let len = u32::from_be_bytes(hdr[..4].try_into().unwrap()) as usize;
        let comp = hdr[4];
        let body = self.data.get(off + 5..off + 4 + len).ok_or_else(|| bad("chunk length out of range"))?;
        let mut raw = Vec::new();
        match comp {
            1 => { GzDecoder::new(body).read_to_end(&mut raw)?; }
            2 => { ZlibDecoder::new(body).read_to_end(&mut raw)?; }
            3 => raw.extend_from_slice(body),
            4 => return Err(bad("LZ4 chunk compression not supported yet")),
            _ => return Err(bad("unknown compression")),
        }
        Ok(Some(nbt::read_root(&mut &raw[..])?.1))
    }
}

/// Builds a region file from chunks (zlib). Chunks are (lx, lz, root tag).
pub fn write_region(chunks: &[(usize, usize, Tag)]) -> io::Result<Vec<u8>> {
    let mut out = vec![0u8; SECTOR * 2];
    for (lx, lz, tag) in chunks {
        let mut raw = Vec::new();
        nbt::write_root(&mut raw, "", tag)?;
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&raw)?;
        let z = enc.finish()?;
        let sector = out.len() / SECTOR;
        out.extend_from_slice(&((z.len() + 1) as u32).to_be_bytes());
        out.push(2);
        out.extend_from_slice(&z);
        let sectors = (out.len() - sector * SECTOR).div_ceil(SECTOR);
        out.resize((sector + sectors) * SECTOR, 0);
        let i = (lx & 31) + (lz & 31) * 32;
        let e = ((sector as u32) << 8) | sectors as u32;
        out[i * 4..i * 4 + 4].copy_from_slice(&e.to_be_bytes());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn region_roundtrip() {
        let mut m = BTreeMap::new();
        m.insert("xPos".into(), Tag::Int(3));
        let t = Tag::Compound(m);
        let bytes = write_region(&[(3, 7, t.clone())]).unwrap();
        let r = Region::from_bytes(bytes);
        assert_eq!(r.read_chunk(3, 7).unwrap(), Some(t));
        assert_eq!(r.read_chunk(0, 0).unwrap(), None);
    }
}
