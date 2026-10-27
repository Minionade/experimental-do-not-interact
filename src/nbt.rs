//! NBT (big-endian, Java edition) reader/writer.
use std::collections::BTreeMap;
use std::io::{self, Read, Write};

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Tag>),
    Compound(BTreeMap<String, Tag>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => 1, Tag::Short(_) => 2, Tag::Int(_) => 3, Tag::Long(_) => 4,
            Tag::Float(_) => 5, Tag::Double(_) => 6, Tag::ByteArray(_) => 7, Tag::String(_) => 8,
            Tag::List(_) => 9, Tag::Compound(_) => 10, Tag::IntArray(_) => 11, Tag::LongArray(_) => 12,
        }
    }
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self { Tag::Compound(m) => m.get(key), _ => None }
    }
    pub fn as_str(&self) -> Option<&str> { if let Tag::String(s) = self { Some(s) } else { None } }
    pub fn as_i64(&self) -> Option<i64> {
        match *self { Tag::Byte(v) => Some(v as i64), Tag::Short(v) => Some(v as i64),
            Tag::Int(v) => Some(v as i64), Tag::Long(v) => Some(v), _ => None }
    }
    pub fn as_list(&self) -> Option<&[Tag]> { if let Tag::List(l) = self { Some(l) } else { None } }
    pub fn as_longs(&self) -> Option<&[i64]> { if let Tag::LongArray(l) = self { Some(l) } else { None } }
}

fn bad(msg: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, msg.to_string()) }

fn r_u8(r: &mut impl Read) -> io::Result<u8> { let mut b = [0; 1]; r.read_exact(&mut b)?; Ok(b[0]) }
fn r_i16(r: &mut impl Read) -> io::Result<i16> { let mut b = [0; 2]; r.read_exact(&mut b)?; Ok(i16::from_be_bytes(b)) }
fn r_i32(r: &mut impl Read) -> io::Result<i32> { let mut b = [0; 4]; r.read_exact(&mut b)?; Ok(i32::from_be_bytes(b)) }
fn r_i64(r: &mut impl Read) -> io::Result<i64> { let mut b = [0; 8]; r.read_exact(&mut b)?; Ok(i64::from_be_bytes(b)) }

fn r_string(r: &mut impl Read) -> io::Result<String> {
    let n = r_i16(r)? as u16 as usize;
    let mut b = vec![0; n];
    r.read_exact(&mut b)?;
    // Java uses modified UTF-8; identical to UTF-8 for everything but NUL/supplementary chars.
    Ok(String::from_utf8_lossy(&b).into_owned())
}

fn r_len(r: &mut impl Read) -> io::Result<usize> {
    let n = r_i32(r)?;
    if n < 0 { return Err(bad("negative length")); }
    Ok(n as usize)
}

fn read_payload(r: &mut impl Read, id: u8, depth: u32) -> io::Result<Tag> {
    if depth > 512 { return Err(bad("nbt too deep")); }
    Ok(match id {
        1 => Tag::Byte(r_u8(r)? as i8),
        2 => Tag::Short(r_i16(r)?),
        3 => Tag::Int(r_i32(r)?),
        4 => Tag::Long(r_i64(r)?),
        5 => Tag::Float(f32::from_bits(r_i32(r)? as u32)),
        6 => Tag::Double(f64::from_bits(r_i64(r)? as u64)),
        7 => { let n = r_len(r)?; let mut b = vec![0u8; n]; r.read_exact(&mut b)?;
               Tag::ByteArray(b.into_iter().map(|x| x as i8).collect()) }
        8 => Tag::String(r_string(r)?),
        9 => {
            let t = r_u8(r)?; let n = r_len(r)?;
            if t == 0 && n > 0 { return Err(bad("list of end tags")); }
            let mut v = Vec::with_capacity(n.min(1 << 16));
            for _ in 0..n { v.push(read_payload(r, t, depth + 1)?); }
            Tag::List(v)
        }
        10 => {
            let mut m = BTreeMap::new();
            loop {
                let t = r_u8(r)?;
                if t == 0 { break; }
                let k = r_string(r)?;
                m.insert(k, read_payload(r, t, depth + 1)?);
            }
            Tag::Compound(m)
        }
        11 => { let n = r_len(r)?; let mut v = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n { v.push(r_i32(r)?); } Tag::IntArray(v) }
        12 => { let n = r_len(r)?; let mut v = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n { v.push(r_i64(r)?); } Tag::LongArray(v) }
        _ => return Err(bad("unknown tag id")),
    })
}

/// Reads a root named compound. Returns (root name, tag).
pub fn read_root(r: &mut impl Read) -> io::Result<(String, Tag)> {
    let id = r_u8(r)?;
    if id != 10 { return Err(bad("root is not a compound")); }
    let name = r_string(r)?;
    Ok((name, read_payload(r, id, 0)?))
}

fn w_string(w: &mut impl Write, s: &str) -> io::Result<()> {
    w.write_all(&(s.len() as u16).to_be_bytes())?; w.write_all(s.as_bytes())
}

fn write_payload(w: &mut impl Write, t: &Tag) -> io::Result<()> {
    match t {
        Tag::Byte(v) => w.write_all(&[*v as u8]),
        Tag::Short(v) => w.write_all(&v.to_be_bytes()),
        Tag::Int(v) => w.write_all(&v.to_be_bytes()),
        Tag::Long(v) => w.write_all(&v.to_be_bytes()),
        Tag::Float(v) => w.write_all(&v.to_bits().to_be_bytes()),
        Tag::Double(v) => w.write_all(&v.to_bits().to_be_bytes()),
        Tag::ByteArray(v) => { w.write_all(&(v.len() as i32).to_be_bytes())?;
            w.write_all(&v.iter().map(|&b| b as u8).collect::<Vec<_>>()) }
        Tag::String(s) => w_string(w, s),
        Tag::List(v) => {
            w.write_all(&[v.first().map_or(0, Tag::id)])?;
            w.write_all(&(v.len() as i32).to_be_bytes())?;
            v.iter().try_for_each(|x| write_payload(w, x))
        }
        Tag::Compound(m) => {
            for (k, v) in m { w.write_all(&[v.id()])?; w_string(w, k)?; write_payload(w, v)?; }
            w.write_all(&[0])
        }
        Tag::IntArray(v) => { w.write_all(&(v.len() as i32).to_be_bytes())?;
            v.iter().try_for_each(|x| w.write_all(&x.to_be_bytes())) }
        Tag::LongArray(v) => { w.write_all(&(v.len() as i32).to_be_bytes())?;
            v.iter().try_for_each(|x| w.write_all(&x.to_be_bytes())) }
    }
}

pub fn write_root(w: &mut impl Write, name: &str, t: &Tag) -> io::Result<()> {
    w.write_all(&[10])?; w_string(w, name)?; write_payload(w, t)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let mut m = BTreeMap::new();
        m.insert("a".into(), Tag::Int(5));
        m.insert("l".into(), Tag::LongArray(vec![1, -2]));
        m.insert("s".into(), Tag::List(vec![Tag::String("x".into())]));
        let t = Tag::Compound(m);
        let mut buf = vec![];
        write_root(&mut buf, "root", &t).unwrap();
        let (n, back) = read_root(&mut &buf[..]).unwrap();
        assert_eq!(n, "root");
        assert_eq!(back, t);
    }
}
