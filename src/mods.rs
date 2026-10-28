//! Data-driven block registry plus a native Rust lifecycle seam. No Java mod loader.
use crate::physics::BodyHandle;
use glam::DVec3;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockDefinition {
    pub id: String,
    pub density: f64,
    pub color: [f32; 3],
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModManifest {
    pub api: u32,
    pub id: String,
    #[serde(default)]
    pub blocks: Vec<BlockDefinition>,
}
#[derive(Default)]
pub struct Registry {
    blocks: BTreeMap<String, BlockDefinition>,
}
impl Registry {
    pub fn load_toml(&mut self, text: &str) -> Result<(), String> {
        let manifest: ModManifest = toml::from_str(text).map_err(|e| e.to_string())?;
        if manifest.api != 1 {
            return Err("unsupported mod API; expected 1".into());
        }
        if !identifier(&manifest.id) || manifest.id.contains(':') {
            return Err("invalid mod namespace".into());
        }
        let mut additions = BTreeMap::new();
        for block in manifest.blocks {
            if !identifier(&block.id) || !block.id.starts_with(&format!("{}:", manifest.id)) {
                return Err(format!("block {} must use the mod namespace", block.id));
            }
            if !block.density.is_finite()
                || block.density <= 0.0
                || block
                    .color
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            {
                return Err(format!("invalid material values for {}", block.id));
            }
            if self.blocks.contains_key(&block.id) || additions.contains_key(&block.id) {
                return Err(format!("duplicate block {}", block.id));
            }
            additions.insert(block.id.clone(), block);
        }
        self.blocks.extend(additions);
        Ok(())
    }
    pub fn get(&self, id: &str) -> Option<&BlockDefinition> {
        self.blocks.get(id)
    }
    pub fn len(&self) -> usize {
        self.blocks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_:.-/".contains(&b))
        && s.matches(':').count() <= 1
        && !s.ends_with(':')
}

#[derive(Clone, Debug)]
pub enum Event {
    FixedTick { tick: u64, dt: f64 },
    StructureSpawned(BodyHandle),
}
#[derive(Clone, Debug)]
pub enum Command {
    Accelerate {
        body: BodyHandle,
        acceleration: DVec3,
    },
}
pub trait NativeMod: Send {
    fn on_event(&mut self, event: &Event, commands: &mut Vec<Command>);
}
#[derive(Default)]
pub struct ModHost {
    modules: Vec<Box<dyn NativeMod>>,
}
impl ModHost {
    pub fn register(&mut self, module: impl NativeMod + 'static) {
        self.modules.push(Box::new(module));
    }
    pub fn dispatch(&mut self, event: &Event) -> Vec<Command> {
        let mut commands = Vec::new();
        for module in &mut self.modules {
            module.on_event(event, &mut commands);
        }
        commands
    }
}
