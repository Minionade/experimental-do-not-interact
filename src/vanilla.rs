//! Read-only import of exact, hash-pinned Minecraft resources.
//! Resource loading is not an implementation of vanilla gameplay.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
use zip::ZipArchive;
const MAX_RESOURCE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct VanillaTarget {
    pub version: String,
    pub client_sha1: String,
    pub client_size: u64,
    pub asset_index_id: String,
    pub asset_index_sha1: String,
    pub reports_sha1: BTreeMap<String, String>,
    pub item_components_sha1: String,
    pub physics_sha1: String,
}
impl VanillaTarget {
    pub fn pinned() -> Self {
        serde_json::from_str(include_str!("../config/minecraft-26.3.json"))
            .expect("compiled version pin")
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct ResourceId {
    namespace: String,
    path: String,
}
impl ResourceId {
    pub fn parse(s: &str) -> Result<Self, String> {
        let (namespace, path) = s.split_once(':').unwrap_or(("minecraft", s));
        if namespace.is_empty()
            || path.is_empty()
            || !namespace
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
            || !path
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"/_.-".contains(&c))
            || path
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err(format!("invalid resource location {s}"));
        }
        Ok(Self {
            namespace: namespace.into(),
            path: path.into(),
        })
    }
    pub fn key(&self) -> String {
        format!("{}:{}", self.namespace, self.path)
    }
    fn asset_path(&self, directory: &str, extension: &str) -> String {
        format!(
            "assets/{}/{}/{}.{}",
            self.namespace, directory, self.path, extension
        )
    }
    fn translation_key(&self, kind: &str) -> String {
        format!("{kind}.{}.{}", self.namespace, self.path.replace('/', "."))
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct AssetCounts {
    pub blockstate_files: usize,
    pub item_definition_files: usize,
    pub model_files: usize,
    pub texture_files: usize,
    pub entity_texture_files: usize,
    pub data_files: usize,
}
pub struct ClientAssets {
    archive: ZipArchive<File>,
    paths: BTreeSet<String>,
    language: BTreeMap<String, String>,
    pub counts: AssetCounts,
}
impl ClientAssets {
    pub fn open(path: &Path) -> Result<Self, String> {
        let target = VanillaTarget::pinned();
        let mut file = File::open(path).map_err(|e| format!("client JAR: {e}"))?;
        if file.metadata().map_err(|e| e.to_string())?.len() != target.client_size {
            return Err(format!(
                "client size differs from pinned Minecraft {}",
                target.version
            ));
        }
        let mut hash = Sha1::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        if format!("{:x}", hash.finalize()) != target.client_sha1 {
            return Err(format!(
                "client SHA-1 differs from pinned Minecraft {}",
                target.version
            ));
        }
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        Self::read_archive(file, &target.version)
    }
    fn read_archive(file: File, version: &str) -> Result<Self, String> {
        let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut paths = BTreeSet::new();
        let mut counts = AssetCounts::default();
        for i in 0..archive.len() {
            let file = archive.by_index(i).map_err(|e| e.to_string())?;
            let name = file.name();
            if file.enclosed_name().is_none() {
                return Err("invalid archive entry path".into());
            }
            if file.is_dir() {
                continue;
            }
            if !paths.insert(name.to_string()) {
                return Err(format!("duplicate archive resource {name}"));
            }
            if name.starts_with("assets/minecraft/blockstates/") && name.ends_with(".json") {
                counts.blockstate_files += 1;
            }
            if name.starts_with("assets/minecraft/items/") && name.ends_with(".json") {
                counts.item_definition_files += 1;
            }
            if name.starts_with("assets/minecraft/models/") && name.ends_with(".json") {
                counts.model_files += 1;
            }
            if name.starts_with("assets/minecraft/textures/") && name.ends_with(".png") {
                counts.texture_files += 1;
            }
            if name.starts_with("assets/minecraft/textures/entity/") && name.ends_with(".png") {
                counts.entity_texture_files += 1;
            }
            if name.starts_with("data/minecraft/") {
                counts.data_files += 1;
            }
        }
        let mut assets = Self {
            archive,
            paths,
            language: BTreeMap::new(),
            counts,
        };
        if assets.json("version.json")?["id"].as_str() != Some(version) {
            return Err("embedded Minecraft version mismatch".into());
        }
        assets.language =
            serde_json::from_slice(&assets.bytes("assets/minecraft/lang/en_us.json")?)
                .map_err(|e| e.to_string())?;
        Ok(assets)
    }
    fn bytes(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let entry = self
            .archive
            .by_name(path)
            .map_err(|_| format!("missing vanilla resource {path}"))?;
        if entry.size() > MAX_RESOURCE_BYTES {
            return Err("resource exceeds size limit".into());
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry
            .take(MAX_RESOURCE_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        if data.len() as u64 > MAX_RESOURCE_BYTES {
            return Err("resource exceeds size limit".into());
        }
        Ok(data)
    }
    fn json(&mut self, path: &str) -> Result<Value, String> {
        serde_json::from_slice(&self.bytes(path)?).map_err(|e| format!("{path}: {e}"))
    }
    pub fn display_name(&self, kind: &str, id: &ResourceId) -> Result<&str, String> {
        if !["block", "item", "entity"].contains(&kind) {
            return Err("name kind must be block, item, or entity".into());
        }
        let key = id.translation_key(kind);
        self.language
            .get(&key)
            .or_else(|| {
                if kind == "item" {
                    self.language.get(&id.translation_key("block"))
                } else {
                    None
                }
            })
            .map(String::as_str)
            .ok_or_else(|| format!("missing vanilla translation {key}"))
    }
    pub fn blockstate(&mut self, id: &ResourceId) -> Result<Value, String> {
        self.json(&id.asset_path("blockstates", "json"))
    }
    pub fn item_definition(&mut self, id: &ResourceId) -> Result<Value, String> {
        self.json(&id.asset_path("items", "json"))
    }
    pub fn texture_png(&mut self, id: &ResourceId) -> Result<Vec<u8>, String> {
        let bytes = self.bytes(&id.asset_path("textures", "png"))?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("texture does not contain PNG data".into());
        }
        Ok(bytes)
    }
    pub fn texture_metadata(&mut self, id: &ResourceId) -> Result<Option<Value>, String> {
        let path = id.asset_path("textures", "png.mcmeta");
        if self.paths.contains(&path) {
            self.json(&path).map(Some)
        } else {
            Ok(None)
        }
    }
    pub fn data_json(&mut self, directory: &str, id: &ResourceId) -> Result<Value, String> {
        ResourceId::parse(&format!("minecraft:{directory}"))?;
        self.json(&format!(
            "data/{}/{}/{}.json",
            id.namespace, directory, id.path
        ))
    }
    pub fn resource_paths(&self) -> impl Iterator<Item = &str> {
        self.paths
            .iter()
            .filter(|p| p.starts_with("assets/") || p.starts_with("data/"))
            .map(String::as_str)
    }
    pub fn audit_models(&mut self) -> Result<(usize, usize), String> {
        let names: Vec<_> = self
            .paths
            .iter()
            .filter_map(|p| {
                p.strip_prefix("assets/minecraft/models/")
                    .and_then(|p| p.strip_suffix(".json"))
                    .map(str::to_string)
            })
            .collect();
        let mut special = 0;
        for name in &names {
            if self
                .resolve_model(&ResourceId::parse(name)?)?
                .builtin_parent
                .is_some()
            {
                special += 1;
            }
        }
        Ok((names.len(), special))
    }
    /// Resolve inheritance and raw elements. Builtin roots remain explicit requirements.
    pub fn resolve_model(&mut self, id: &ResourceId) -> Result<ResolvedModel, String> {
        self.model_inner(id, &mut BTreeSet::new(), 0)
    }
    fn model_inner(
        &mut self,
        id: &ResourceId,
        visiting: &mut BTreeSet<ResourceId>,
        depth: usize,
    ) -> Result<ResolvedModel, String> {
        if depth >= 64 || !visiting.insert(id.clone()) {
            return Err("model inheritance cycle/depth limit".into());
        }
        let raw = self.json(&id.asset_path("models", "json"))?;
        let mut result = if let Some(parent) = raw.get("parent") {
            let parent =
                ResourceId::parse(parent.as_str().ok_or("model parent must be a string")?)?;
            if parent.namespace == "minecraft" && parent.path.starts_with("builtin/") {
                ResolvedModel {
                    builtin_parent: Some(parent.key()),
                    ..Default::default()
                }
            } else {
                self.model_inner(&parent, visiting, depth + 1)?
            }
        } else {
            ResolvedModel::default()
        };
        if let Some(textures) = raw.get("textures") {
            for (key, value) in textures
                .as_object()
                .ok_or("model textures must be an object")?
            {
                if !value.is_string() && value.get("sprite").and_then(Value::as_str).is_none() {
                    return Err("model texture must be a reference or sprite object".into());
                }
                result.textures.insert(key.clone(), value.clone());
            }
        }
        if let Some(elements) = raw.get("elements") {
            result.elements = Some(
                elements
                    .as_array()
                    .ok_or("model elements must be an array")?
                    .clone(),
            );
        }
        if let Some(ao) = raw.get("ambientocclusion") {
            result.ambient_occlusion =
                Some(ao.as_bool().ok_or("ambientocclusion must be boolean")?);
        }
        result.inheritance.push((id.key(), raw));
        visiting.remove(id);
        Ok(result)
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct ResolvedModel {
    pub textures: BTreeMap<String, Value>,
    pub elements: Option<Vec<Value>>,
    pub ambient_occlusion: Option<bool>,
    pub builtin_parent: Option<String>,
    pub inheritance: Vec<(String, Value)>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedTexture {
    pub sprite: ResourceId,
    pub metadata: Value,
}
impl ResolvedModel {
    pub fn texture_binding(&self, reference: &str) -> Result<ResolvedTexture, String> {
        let mut current = reference;
        let mut seen = BTreeSet::new();
        while let Some(key) = current.strip_prefix('#') {
            if !seen.insert(key) {
                return Err("texture reference cycle".into());
            }
            let value = self
                .textures
                .get(key)
                .ok_or_else(|| format!("missing texture reference #{key}"))?;
            if let Some(text) = value.as_str() {
                current = text;
            } else {
                let sprite = value
                    .get("sprite")
                    .and_then(Value::as_str)
                    .ok_or("texture object requires sprite")?;
                return Ok(ResolvedTexture {
                    sprite: ResourceId::parse(sprite)?,
                    metadata: value.clone(),
                });
            }
        }
        Ok(ResolvedTexture {
            sprite: ResourceId::parse(current)?,
            metadata: Value::Null,
        })
    }
    pub fn texture(&self, reference: &str) -> Result<ResourceId, String> {
        Ok(self.texture_binding(reference)?.sprite)
    }
}

/// Parse authoritative report IDs without guessing from resource filenames.
/// Schema validation alone does not certify report version or provenance.
pub struct RegistryReports {
    pub registries: Value,
    pub blocks: Value,
    pub items: Option<Value>,
}
impl RegistryReports {
    /// Exact hashes generated from the pinned official server.
    pub fn load_pinned(directory: &Path) -> Result<Self, String> {
        let target = VanillaTarget::pinned();
        let read = |name: &str| -> Result<Value, String> {
            let path = directory.join(name);
            let file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if file.metadata().map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
                return Err("report exceeds size limit".into());
            }
            let mut bytes = Vec::new();
            file.take(128 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if target.reports_sha1.get(name) != Some(&format!("{:x}", Sha1::digest(&bytes))) {
                return Err(format!(
                    "{name} differs from pinned {} report",
                    target.version
                ));
            }
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())
        };
        Self::validate(read("registries.json")?, read("blocks.json")?, None)
    }
    pub fn load(directory: &Path) -> Result<Self, String> {
        let read = |name: &str| -> Result<Value, String> {
            let p = directory.join(name);
            let file = File::open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            if file.metadata().map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
                return Err("report exceeds size limit".into());
            }
            serde_json::from_reader(file).map_err(|e| e.to_string())
        };
        let registries = read("registries.json")?;
        let blocks = read("blocks.json")?;
        let items = if directory.join("items.json").exists() {
            Some(read("items.json")?)
        } else {
            None
        };
        Self::validate(registries, blocks, items)
    }
    fn validate(registries: Value, blocks: Value, items: Option<Value>) -> Result<Self, String> {
        for registry in ["minecraft:block", "minecraft:item", "minecraft:entity_type"] {
            let entries = registries[registry]["entries"]
                .as_object()
                .ok_or_else(|| format!("missing {registry} entries"))?;
            let mut ids = BTreeSet::new();
            for (name, value) in entries {
                ResourceId::parse(name)?;
                let id = value["protocol_id"]
                    .as_u64()
                    .filter(|n| *n <= u32::MAX as u64)
                    .ok_or("invalid registry protocol id")?;
                if !ids.insert(id) {
                    return Err(format!("duplicate protocol ID in {registry}"));
                }
            }
        }
        let definitions = blocks
            .as_object()
            .ok_or("blocks report must be an object")?;
        let mut states = BTreeSet::new();
        for (name, value) in definitions {
            ResourceId::parse(name)?;
            if registries["minecraft:block"]["entries"].get(name).is_none() {
                return Err(format!("unregistered block in states report {name}"));
            }
            let values = value["states"].as_array().ok_or("missing block states")?;
            if values.is_empty()
                || values
                    .iter()
                    .filter(|v| v["default"].as_bool() == Some(true))
                    .count()
                    != 1
            {
                return Err(format!("{name} must have one default state"));
            }
            for state in values {
                let id = state["id"]
                    .as_u64()
                    .filter(|n| *n <= u32::MAX as u64)
                    .ok_or("invalid block state id")?;
                if !states.insert(id) {
                    return Err("duplicate global block state ID".into());
                }
            }
        }
        if definitions.len()
            != registries["minecraft:block"]["entries"]
                .as_object()
                .unwrap()
                .len()
        {
            return Err("block registry/report roster mismatch".into());
        }
        Ok(Self {
            registries,
            blocks,
            items,
        })
    }
    pub fn protocol_id(&self, registry: &str, id: &ResourceId) -> Option<u32> {
        self.registries[registry]["entries"][id.key()]["protocol_id"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
    }
    pub fn default_block_state(&self, id: &ResourceId) -> Option<u32> {
        self.blocks[id.key()]["states"]
            .as_array()?
            .iter()
            .find(|s| s["default"].as_bool() == Some(true))?["id"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn fixture(entries: &[(&str, &str)], tag: &str) -> (ClientAssets, std::path::PathBuf) {
        let path =
            std::env::temp_dir().join(format!("orbital-assets-{}-{tag}.zip", std::process::id()));
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name,value) in [("version.json",r#"{"id":"26.3"}"#),("assets/minecraft/lang/en_us.json",r#"{"block.minecraft.stone":"Stone","item.minecraft.diamond_pickaxe":"Diamond Pickaxe","entity.minecraft.zombie":"Zombie"}"#)].iter().chain(entries.iter()){
            zip.start_file(*name,zip::write::FileOptions::default()).unwrap();zip.write_all(value.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let assets = ClientAssets::read_archive(File::open(&path).unwrap(), "26.3").unwrap();
        (assets, path)
    }
    #[test]
    fn exact_names_and_default_namespace() {
        let (a, p) = fixture(&[], "names");
        assert_eq!(
            a.display_name("item", &ResourceId::parse("diamond_pickaxe").unwrap())
                .unwrap(),
            "Diamond Pickaxe"
        );
        assert_eq!(
            a.display_name("item", &ResourceId::parse("stone").unwrap())
                .unwrap(),
            "Stone"
        );
        assert_eq!(
            a.display_name("entity", &ResourceId::parse("zombie").unwrap())
                .unwrap(),
            "Zombie"
        );
        drop(a);
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn model_inheritance_preserves_aliases_and_fields() {
        let (mut a, p) = fixture(
            &[
                (
                    "assets/minecraft/models/block/base.json",
                    r##"{"textures":{"all":"minecraft:block/stone","side":"#all"},"elements":[{"from":[0,0,0],"to":[16,16,16]}]}"##,
                ),
                (
                    "assets/minecraft/models/block/child.json",
                    r##"{"parent":"minecraft:block/base","textures":{"all":"minecraft:block/dirt"},"new_field_263":true}"##,
                ),
            ],
            "inherit",
        );
        let m = a
            .resolve_model(&ResourceId::parse("block/child").unwrap())
            .unwrap();
        assert_eq!(m.texture("#side").unwrap().key(), "minecraft:block/dirt");
        assert_eq!(m.elements.as_ref().unwrap().len(), 1);
        assert_eq!(m.inheritance[1].1["new_field_263"], true);
        drop(a);
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn model_cycles_fail() {
        let (mut a, p) = fixture(
            &[
                (
                    "assets/minecraft/models/block/a.json",
                    r#"{"parent":"block/b"}"#,
                ),
                (
                    "assets/minecraft/models/block/b.json",
                    r#"{"parent":"block/a"}"#,
                ),
            ],
            "cycle",
        );
        assert!(a
            .resolve_model(&ResourceId::parse("block/a").unwrap())
            .is_err());
        drop(a);
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn generated_items_require_special_renderer() {
        let (mut a, p) = fixture(
            &[(
                "assets/minecraft/models/item/generated.json",
                r#"{"parent":"builtin/generated"}"#,
            )],
            "generated",
        );
        assert_eq!(
            a.resolve_model(&ResourceId::parse("item/generated").unwrap())
                .unwrap()
                .builtin_parent
                .as_deref(),
            Some("minecraft:builtin/generated")
        );
        drop(a);
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn sprite_objects_preserve_translucency_metadata() {
        let m = ResolvedModel {
            textures: BTreeMap::from([
                (
                    "layer".into(),
                    serde_json::json!({"sprite":"block/redstone_dust_dot","force_translucent":true}),
                ),
                ("alias".into(), serde_json::json!("#layer")),
            ]),
            ..Default::default()
        };
        let b = m.texture_binding("#alias").unwrap();
        assert_eq!(b.sprite.key(), "minecraft:block/redstone_dust_dot");
        assert_eq!(b.metadata["force_translucent"], true);
    }
    #[test]
    fn texture_cycles_fail() {
        let m = ResolvedModel {
            textures: BTreeMap::from([("a".into(), "#b".into()), ("b".into(), "#a".into())]),
            ..Default::default()
        };
        assert!(m.texture("#a").is_err());
    }
    #[test]
    fn invalid_resource_ids_fail() {
        for id in [
            "../stone",
            "minecraft:/a",
            "minecraft:a//b",
            "../evil:path",
            "minecraft:A",
            "minecraft:a:b",
            "minecraft:a/../b",
        ] {
            assert!(ResourceId::parse(id).is_err(), "{id}");
        }
    }
    #[test]
    fn production_loader_rejects_unpinned_fixture() {
        let (a, p) = fixture(&[], "pin");
        drop(a);
        assert!(ClientAssets::open(&p).is_err());
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn report_ids_are_not_texture_indices() {
        let registry = serde_json::json!({"minecraft:block":{"entries":{"minecraft:stone":{"protocol_id":7}}},"minecraft:item":{"entries":{"minecraft:stone":{"protocol_id":42}}},"minecraft:entity_type":{"entries":{"minecraft:zombie":{"protocol_id":90}}}});
        let blocks =
            serde_json::json!({"minecraft:stone":{"states":[{"id":90001,"default":true}]}});
        let r = RegistryReports::validate(registry, blocks, None).unwrap();
        let stone = ResourceId::parse("stone").unwrap();
        assert_eq!(r.protocol_id("minecraft:item", &stone), Some(42));
        assert_eq!(r.default_block_state(&stone), Some(90001));
    }
}
