//! Minecraft resource pack access: directories, zips and jars, stacked by priority.
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zip::ZipArchive;

enum Source {
    Dir(PathBuf),
    Zip { archive: Mutex<ZipArchive<File>>, root: String, names: Vec<String> },
}

pub struct Pack { pub name: String, source: Source }

fn invalid(msg: String) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, msg) }

impl Pack {
    /// Opens a pack folder, .zip or client .jar. A pack wrapped in one extra top-level folder is accepted.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Pack> {
        let path = path.as_ref();
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        if path.is_dir() {
            let mut root = path.to_path_buf();
            if !root.join("pack.mcmeta").exists() && !root.join("assets").exists() {
                // One level of wrapping folder.
                let inner: Vec<_> = std::fs::read_dir(&root)?.filter_map(Result::ok)
                    .filter(|e| e.path().join("pack.mcmeta").exists() || e.path().join("assets").exists()).collect();
                if inner.len() == 1 { root = inner[0].path(); }
            }
            return Ok(Pack { name, source: Source::Dir(root) });
        }
        let mut archive = ZipArchive::new(File::open(path)?).map_err(|e| invalid(format!("{}: {e}", path.display())))?;
        let names: Vec<String> = archive.file_names().filter_map(|n| n.ok().map(|c| c.into_owned())).collect();
        let root = if names.iter().any(|n| n == "pack.mcmeta" || n.starts_with("assets/")) { String::new() }
            else if let Some(n) = names.iter().find(|n| n.matches('/').count() == 1 && n.ends_with("/pack.mcmeta")) {
                n[..n.len() - "pack.mcmeta".len()].to_string()
            } else if let Some(n) = names.iter().find(|n| n.matches('/').count() >= 1 && n.split('/').nth(1) == Some("assets")) {
                format!("{}/", n.split('/').next().unwrap())
            } else { String::new() };
        let _ = &mut archive;
        Ok(Pack { name, source: Source::Zip { archive: Mutex::new(archive), root, names } })
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        match &self.source {
            Source::Dir(root) => {
                if path.split('/').any(|c| c == ".." || c.is_empty()) { return None; }
                std::fs::read(root.join(path)).ok()
            }
            Source::Zip { archive, root, .. } => {
                let mut a = archive.lock().unwrap();
                let mut f = a.by_name(&format!("{root}{path}")).ok()?;
                let mut buf = Vec::with_capacity(f.size() as usize);
                f.read_to_end(&mut buf).ok()?;
                Some(buf)
            }
        }
    }

    /// Files whose pack-relative path starts with `prefix` (e.g. "assets/minecraft/textures/block/").
    pub fn list(&self, prefix: &str) -> Vec<String> {
        match &self.source {
            Source::Dir(root) => {
                let mut out = Vec::new();
                fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
                    let Ok(rd) = std::fs::read_dir(dir) else { return };
                    for e in rd.filter_map(Result::ok) {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
                        if e.path().is_dir() { walk(&e.path(), &rel, out); } else { out.push(rel); }
                    }
                }
                walk(root, "", &mut out);
                out.retain(|p| p.starts_with(prefix));
                out
            }
            Source::Zip { root, names, .. } => names.iter()
                .filter_map(|n| n.strip_prefix(root.as_str())).filter(|n| n.starts_with(prefix) && !n.ends_with('/'))
                .map(String::from).collect(),
        }
    }
}

/// Packs in priority order: later packs override earlier ones, like Minecraft's resource pack screen (bottom to top).
#[derive(Default)]
pub struct PackStack { packs: Vec<Pack> }

impl PackStack {
    pub fn new() -> Self { Self::default() }
    pub fn push(&mut self, p: Pack) { self.packs.push(p); }
    pub fn is_empty(&self) -> bool { self.packs.is_empty() }
    pub fn names(&self) -> Vec<&str> { self.packs.iter().map(|p| p.name.as_str()).collect() }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> { self.packs.iter().rev().find_map(|p| p.read(path)) }
    pub fn read_json(&self, path: &str) -> Option<serde_json::Value> { serde_json::from_slice(&self.read(path)?).ok() }

    /// Union of files across packs under `prefix` (sorted, deduplicated).
    pub fn list(&self, prefix: &str) -> BTreeSet<String> { self.packs.iter().flat_map(|p| p.list(prefix)).collect() }
}

/// Splits "namespace:path" (default namespace "minecraft").
pub fn split_id(id: &str) -> (&str, &str) { id.split_once(':').unwrap_or(("minecraft", id)) }

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::Write;

    /// Fresh temp dir, unique per call (tests run in parallel inside one process).
    pub fn temp(name: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("be_{name}_{}_{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    pub fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(File::create(path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for (n, b) in files { z.start_file(*n, opts).unwrap(); z.write_all(b).unwrap(); }
        z.finish().unwrap();
    }

    #[test]
    fn dir_zip_and_wrapped_zip_all_read() {
        let d = temp("pack");
        std::fs::create_dir_all(d.join("dirpack/assets/minecraft/textures/block")).unwrap();
        std::fs::write(d.join("dirpack/pack.mcmeta"), "{}").unwrap();
        std::fs::write(d.join("dirpack/assets/minecraft/textures/block/a.png"), b"dir-a").unwrap();
        write_zip(&d.join("plain.zip"), &[("pack.mcmeta", b"{}"), ("assets/minecraft/textures/block/a.png", b"zip-a"), ("assets/minecraft/textures/block/b.png", b"zip-b")]);
        write_zip(&d.join("wrapped.zip"), &[("MyPack/pack.mcmeta", b"{}"), ("MyPack/assets/minecraft/textures/block/c.png", b"wrapped-c")]);
        let mut s = PackStack::new();
        s.push(Pack::open(d.join("plain.zip")).unwrap());
        s.push(Pack::open(d.join("dirpack")).unwrap());
        s.push(Pack::open(d.join("wrapped.zip")).unwrap());
        // Dir pack sits above the zip: it overrides a.png, b.png still comes from the zip.
        assert_eq!(s.read("assets/minecraft/textures/block/a.png").unwrap(), b"dir-a");
        assert_eq!(s.read("assets/minecraft/textures/block/b.png").unwrap(), b"zip-b");
        assert_eq!(s.read("assets/minecraft/textures/block/c.png").unwrap(), b"wrapped-c");
        assert!(s.read("assets/minecraft/textures/block/none.png").is_none());
        assert!(s.read("../etc/passwd").is_none());
        let l = s.list("assets/minecraft/textures/block/");
        assert_eq!(l.len(), 3, "{l:?}");
        assert_eq!(split_id("foo:bar/baz"), ("foo", "bar/baz"));
        assert_eq!(split_id("block/stone"), ("minecraft", "block/stone"));
        std::fs::remove_dir_all(d).ok();
    }
}
