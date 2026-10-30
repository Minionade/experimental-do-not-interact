//! Hash-bound runtime properties exported by tools/VanillaOracle.java.
use crate::{collision::Aabb, state_registry::StateRegistry, vanilla::VanillaTarget};
use glam::DVec3;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::{collections::BTreeMap, path::Path};
#[derive(Clone, Debug, Deserialize)]
pub struct StatePhysics {
    pub id: u32,
    pub block: String,
    pub hardness: f64,
    pub blast_resistance: f64,
    pub friction: f64,
    pub light: u8,
    pub requires_correct_tool: bool,
    pub can_occlude: bool,
    pub dynamic_shape: bool,
    pub position_offset: bool,
    pub outline: Option<Vec<[f64; 6]>>,
    pub collision: Option<Vec<[f64; 6]>>,
    pub occlusion: Option<Vec<[f64; 6]>>,
    pub shape_error: Option<String>,
}
#[derive(Deserialize)]
struct Report {
    format: u32,
    version: String,
    context: String,
    states: Vec<StatePhysics>,
    crafting_remainders: BTreeMap<String, serde_json::Value>,
}
pub struct PhysicsCatalog {
    states: BTreeMap<u32, StatePhysics>,
}
impl PhysicsCatalog {
    pub fn load_pinned(path: &Path, registry: &StateRegistry) -> Result<Self, String> {
        let file = std::fs::File::open(path)
            .map_err(|e| format!("physics report: {e}; regenerate reports with the JDK helper"))?;
        if file.metadata().map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
            return Err("physics report exceeds limit".into());
        }
        let bytes = std::io::read_to_string(file).map_err(|e| e.to_string())?;
        if format!("{:x}", Sha1::digest(bytes.as_bytes())) != VanillaTarget::pinned().physics_sha1 {
            return Err("physics report hash mismatch".into());
        }
        let report: Report = serde_json::from_str(&bytes).map_err(|e| e.to_string())?;
        if report.format != 1
            || report.version != VanillaTarget::pinned().version
            || report.context != "EmptyBlockGetter; BlockPos.ZERO; empty CollisionContext"
        {
            return Err("unsupported physics report context".into());
        }
        let mut states = BTreeMap::new();
        for state in report.states {
            if registry
                .get(state.id)
                .is_none_or(|key| key.block != state.block)
            {
                return Err("physics state identity mismatch".into());
            }
            for boxes in [&state.collision, &state.outline, &state.occlusion]
                .into_iter()
                .flatten()
            {
                for b in boxes {
                    Aabb::new(DVec3::new(b[0], b[1], b[2]), DVec3::new(b[3], b[4], b[5]))?;
                }
            }
            if states.insert(state.id, state).is_some() {
                return Err("duplicate physics state".into());
            }
        }
        if states.len() != registry.len() {
            return Err("incomplete physics roster".into());
        }
        for (item, value) in report.crafting_remainders {
            if value.get("id").and_then(|v| v.as_str())
                != crate::crafting::crafting_remainder(&item)
            {
                return Err(format!("crafting remainder mismatch: {item}"));
            }
        }
        Ok(Self { states })
    }
    pub fn get(&self, id: u32) -> Option<&StatePhysics> {
        self.states.get(&id)
    }
    pub fn static_boxes(&self, id: u32, outline: bool) -> Result<Vec<Aabb>, String> {
        let state = self.get(id).ok_or("unregistered physics state")?;
        if state.dynamic_shape || state.position_offset || state.shape_error.is_some() {
            return Err(format!("{} needs contextual shape evaluation", state.block));
        }
        let boxes = if outline {
            &state.outline
        } else {
            &state.collision
        };
        boxes
            .as_ref()
            .ok_or("missing shape")?
            .iter()
            .map(|b| Aabb::new(DVec3::new(b[0], b[1], b[2]), DVec3::new(b[3], b[4], b[5])))
            .collect()
    }
}
