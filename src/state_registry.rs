//! Authoritative state identity from generated reports, never resource-file indices.
use crate::{
    blockstate::Properties,
    vanilla::{RegistryReports, ResourceId},
};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct StateKey {
    pub block: String,
    pub properties: Properties,
}
pub struct StateRegistry {
    states: BTreeMap<u32, StateKey>,
    reverse: BTreeMap<StateKey, u32>,
    defaults: BTreeMap<String, u32>,
    air: BTreeSet<u32>,
    fingerprint: String,
}
impl StateRegistry {
    pub fn from_reports(reports: &RegistryReports) -> Result<Self, String> {
        Self::from_blocks(&reports.blocks)
    }
    fn from_blocks(blocks: &serde_json::Value) -> Result<Self, String> {
        let mut states = BTreeMap::new();
        let mut reverse = BTreeMap::new();
        let mut defaults = BTreeMap::new();
        let mut air = BTreeSet::new();
        for (block, definition) in blocks.as_object().ok_or("blocks report must be object")? {
            if ResourceId::parse(block)?.key() != *block {
                return Err("report IDs must include namespace".into());
            }
            let domains: BTreeMap<String, Vec<String>> = definition
                .get("properties")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|e| format!("property domains: {e}"))?
                .unwrap_or_default();
            let values = definition["states"]
                .as_array()
                .ok_or("missing block states")?;
            for state in values {
                let id = u32::try_from(state["id"].as_u64().ok_or("invalid state ID")?)
                    .map_err(|_| "state ID overflow")?;
                let properties: Properties = state
                    .get("properties")
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()
                    .map_err(|e| format!("state properties: {e}"))?
                    .unwrap_or_default();
                if properties.len() != domains.len()
                    || properties
                        .iter()
                        .any(|(k, v)| !domains.get(k).is_some_and(|d| d.contains(v)))
                {
                    return Err(format!("invalid properties for state {id}"));
                }
                let key = StateKey {
                    block: block.clone(),
                    properties,
                };
                if states.insert(id, key.clone()).is_some() || reverse.insert(key, id).is_some() {
                    return Err("duplicate ID or state properties".into());
                }
                if state
                    .get("default")
                    .map(|v| v.as_bool().ok_or("default must be boolean"))
                    .transpose()?
                    .unwrap_or(false)
                    && defaults.insert(block.clone(), id).is_some()
                {
                    return Err("multiple default states".into());
                }
                if ["minecraft:air", "minecraft:cave_air", "minecraft:void_air"]
                    .contains(&block.as_str())
                {
                    air.insert(id);
                }
            }
            if !defaults.contains_key(block) {
                return Err(format!("missing default state for {block}"));
            }
            let count = domains.values().try_fold(1usize, |n, d| {
                if d.is_empty() || d.iter().collect::<BTreeSet<_>>().len() != d.len() {
                    return Err("invalid property domain");
                }
                n.checked_mul(d.len()).ok_or("property count overflow")
            })?;
            if values.len() != count {
                return Err(format!("incomplete states for {block}"));
            }
        }
        if states.get(&0).map(|k| k.block.as_str()) != Some("minecraft:air") {
            return Err("octree AIR=0 requires minecraft:air state 0".into());
        }
        let fingerprint = format!(
            "{:x}",
            Sha1::digest(serde_json::to_vec(&states).map_err(|e| e.to_string())?)
        );
        Ok(Self {
            states,
            reverse,
            defaults,
            air,
            fingerprint,
        })
    }
    pub fn get(&self, id: u32) -> Option<&StateKey> {
        self.states.get(&id)
    }
    pub fn len(&self) -> usize {
        self.states.len()
    }
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
    pub fn is_air(&self, id: u32) -> bool {
        self.air.contains(&id)
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn states(&self) -> impl Iterator<Item = (u32, &StateKey)> {
        self.states.iter().map(|(&id, state)| (id, state))
    }
    pub fn resolve(&self, block: &str, overrides: &Properties) -> Result<u32, String> {
        let block = ResourceId::parse(block)?.key();
        let id = self
            .defaults
            .get(&block)
            .ok_or_else(|| format!("unknown block {block}"))?;
        let mut key = self.states[id].clone();
        for (k, v) in overrides {
            if !key.properties.contains_key(k) {
                return Err(format!("unknown property {k} for {block}"));
            }
            key.properties.insert(k.clone(), v.clone());
        }
        self.reverse
            .get(&key)
            .copied()
            .ok_or_else(|| format!("invalid state {key:?}"))
    }
}
#[cfg(test)]
pub(crate) fn fixture() -> StateRegistry {
    StateRegistry::from_blocks(&serde_json::json!({
        "minecraft:air":{"states":[{"id":0,"default":true}]},
        "minecraft:stone":{"states":[{"id":1,"default":true}]},
        "minecraft:furnace":{"properties":{"lit":["false","true"]},"states":[{"id":70000,"default":true,"properties":{"lit":"false"}},{"id":70001,"properties":{"lit":"true"}}]}
    })).unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_ids_and_default_overrides() {
        let r = fixture();
        assert_eq!(r.resolve("furnace", &Properties::new()).unwrap(), 70000);
        assert_eq!(
            r.resolve("furnace", &[("lit".into(), "true".into())].into())
                .unwrap(),
            70001
        );
        assert!(r
            .resolve("furnace", &[("lit".into(), "maybe".into())].into())
            .is_err());
        assert!(r
            .resolve("stone", &[("lit".into(), "true".into())].into())
            .is_err());
    }
    #[test]
    fn incomplete_combinations_fail() {
        assert!(StateRegistry::from_blocks(&serde_json::json!({"minecraft:air":{"states":[{"id":0,"default":true}]},"minecraft:test":{"properties":{"x":["a","b"]},"states":[{"id":1,"default":true,"properties":{"x":"a"}}]}})).is_err());
    }
}
