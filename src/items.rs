//! Verified vanilla item components and lossless stack/inventory operations.
use crate::vanilla::{RegistryReports, ResourceId, VanillaTarget};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{collections::BTreeMap, io::Read, path::Path};
pub struct ItemDefinition {
    pub protocol_id: u32,
    pub components: BTreeMap<String, Value>,
}
pub struct ItemCatalog {
    items: BTreeMap<String, ItemDefinition>,
}
impl ItemCatalog {
    pub fn load_pinned(reports: &RegistryReports, directory: &Path) -> Result<Self, String> {
        let entries = reports.registries["minecraft:item"]["entries"]
            .as_object()
            .ok_or("missing item registry")?;
        let mut hash = Sha1::new();
        let mut items = BTreeMap::new();
        for (name, entry) in entries {
            let name = ResourceId::parse(name)?.key();
            let (ns, path) = name.split_once(':').unwrap();
            let file = std::fs::File::open(
                directory
                    .join(ns)
                    .join("components/item")
                    .join(format!("{path}.json")),
            )
            .map_err(|e| format!("item components {name}: {e}"))?;
            if file.metadata().map_err(|e| e.to_string())?.len() > 1024 * 1024 {
                return Err("item components exceed size limit".into());
            }
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            hash.update(name.as_bytes());
            hash.update([0]);
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(&bytes);
            let raw: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let components: BTreeMap<String, Value> =
                serde_json::from_value(raw["components"].clone()).map_err(|e| e.to_string())?;
            let protocol_id = u32::try_from(
                entry["protocol_id"]
                    .as_u64()
                    .ok_or("invalid item protocol ID")?,
            )
            .map_err(|_| "item protocol ID overflow")?;
            items.insert(
                name,
                ItemDefinition {
                    protocol_id,
                    components,
                },
            );
        }
        if format!("{:x}", hash.finalize()) != VanillaTarget::pinned().item_components_sha1 {
            return Err("item component reports differ from pinned 26.3 defaults".into());
        }
        let catalog = Self { items };
        for name in catalog.items.keys() {
            if name != "minecraft:air" {
                catalog.stack(name, 1)?.validate(&catalog)?;
            }
        }
        Ok(catalog)
    }
    pub fn get(&self, name: &str) -> Option<&ItemDefinition> {
        self.items.get(name)
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.items.keys().map(String::as_str)
    }
    pub fn stack(&self, name: &str, count: u32) -> Result<ItemStack, String> {
        let stack = ItemStack {
            item: ResourceId::parse(name)?.key(),
            count,
            patch: BTreeMap::new(),
        };
        stack.validate(self)?;
        Ok(stack)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemStack {
    pub item: String,
    pub count: u32,
    #[serde(default)]
    pub patch: BTreeMap<String, Option<Value>>,
}
impl ItemStack {
    pub fn component<'a>(&'a self, catalog: &'a ItemCatalog, key: &str) -> Option<&'a Value> {
        match self.patch.get(key) {
            Some(value) => value.as_ref(),
            None => catalog.get(&self.item)?.components.get(key),
        }
    }
    pub fn max_count(&self, catalog: &ItemCatalog) -> Result<u32, String> {
        self.component(catalog, "minecraft:max_stack_size")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0 && *n <= 99)
            .ok_or_else(|| "invalid/missing max_stack_size".into())
    }
    pub fn validate(&self, catalog: &ItemCatalog) -> Result<(), String> {
        if self.item == "minecraft:air" {
            return Err("empty slots must be None, not an air stack".into());
        }
        if catalog.get(&self.item).is_none() {
            return Err(format!("unknown item {}", self.item));
        }
        for key in self.patch.keys() {
            if ResourceId::parse(key)?.key() != *key {
                return Err("component patch key must be namespaced".into());
            }
        }
        let max = self.max_count(catalog)?;
        if self.count == 0 || self.count > max {
            return Err("stack count outside item limit".into());
        }
        let durability = self
            .component(catalog, "minecraft:max_damage")
            .map(|v| v.as_u64().filter(|n| *n > 0).ok_or("invalid max_damage"))
            .transpose()?;
        if durability.is_some() && max != 1 {
            return Err("damageable items must be unstackable".into());
        }
        if let Some(damage) = self.component(catalog, "minecraft:damage") {
            let damage = damage.as_u64().ok_or("invalid item damage")?;
            if damage > durability.ok_or("damage requires max_damage")? {
                return Err("damage exceeds durability".into());
            }
        }
        Ok(())
    }
    pub fn same_kind(&self, other: &Self, catalog: &ItemCatalog) -> bool {
        self.item == other.item
            && self
                .patch
                .keys()
                .chain(other.patch.keys())
                .all(|key| self.component(catalog, key) == other.component(catalog, key))
    }
    /// Returns true when the caller must remove the broken stack. No Unbreaking RNG yet.
    pub fn damage(&mut self, catalog: &ItemCatalog, amount: u32) -> Result<bool, String> {
        self.validate(catalog)?;
        let Some(max) = self
            .component(catalog, "minecraft:max_damage")
            .and_then(Value::as_u64)
        else {
            return Ok(false);
        };
        let current = self
            .component(catalog, "minecraft:damage")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let next = current.saturating_add(amount as u64);
        if next >= max {
            return Ok(true);
        }
        self.patch
            .insert("minecraft:damage".into(), Some(Value::from(next)));
        Ok(false)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Inventory {
    slots: Vec<Option<ItemStack>>,
}
impl Inventory {
    pub fn new(size: usize) -> Result<Self, String> {
        if size > 4096 {
            return Err("inventory size exceeds limit".into());
        }
        Ok(Self {
            slots: vec![None; size],
        })
    }
    pub fn slots(&self) -> &[Option<ItemStack>] {
        &self.slots
    }
    pub fn validate(&self, catalog: &ItemCatalog) -> Result<(), String> {
        if self.slots.len() > 4096 {
            return Err("inventory size exceeds limit".into());
        }
        for s in self.slots.iter().flatten() {
            s.validate(catalog)?;
        }
        Ok(())
    }
    pub fn set(
        &mut self,
        index: usize,
        stack: Option<ItemStack>,
        catalog: &ItemCatalog,
    ) -> Result<(), String> {
        if let Some(s) = &stack {
            s.validate(catalog)?;
        }
        *self.slots.get_mut(index).ok_or("slot outside inventory")? = stack;
        Ok(())
    }
    /// Merge compatible stacks first, then vacant slots. Return an uninserted remainder.
    pub fn insert(
        &mut self,
        mut incoming: ItemStack,
        catalog: &ItemCatalog,
    ) -> Result<Option<ItemStack>, String> {
        incoming.validate(catalog)?;
        self.validate(catalog)?;
        let max = incoming.max_count(catalog)?;
        for target in self.slots.iter_mut().flatten() {
            if target.same_kind(&incoming, catalog) {
                let n = incoming.count.min(max - target.count);
                target.count += n;
                incoming.count -= n;
                if incoming.count == 0 {
                    return Ok(None);
                }
            }
        }
        for slot in &mut self.slots {
            if slot.is_none() {
                *slot = Some(incoming);
                return Ok(None);
            }
        }
        Ok(Some(incoming))
    }
    pub fn remove(&mut self, index: usize, count: u32) -> Result<Option<ItemStack>, String> {
        let slot = self.slots.get_mut(index).ok_or("slot outside inventory")?;
        let Some(stack) = slot else {
            return Ok(None);
        };
        if count == 0 {
            return Ok(None);
        }
        let mut taken = stack.clone();
        taken.count = count.min(stack.count);
        stack.count -= taken.count;
        if stack.count == 0 {
            *slot = None;
        }
        Ok(Some(taken))
    }
}
#[cfg(test)]
pub(crate) fn fixture() -> ItemCatalog {
    let mut items = BTreeMap::new();
    for (name, max) in [
        ("stone", 64),
        ("stick", 64),
        ("oak_planks", 64),
        ("iron_ingot", 64),
        ("iron_pickaxe", 1),
        ("bucket", 16),
        ("milk_bucket", 1),
    ] {
        let mut components: BTreeMap<String, Value> =
            [("minecraft:max_stack_size".into(), Value::from(max))].into();
        if name == "iron_pickaxe" {
            components.insert("minecraft:max_damage".into(), Value::from(250));
            components.insert("minecraft:damage".into(), Value::from(0));
        }
        items.insert(
            format!("minecraft:{name}"),
            ItemDefinition {
                protocol_id: items.len() as u32,
                components,
            },
        );
    }
    ItemCatalog { items }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_split_and_full_inventory_conserve_items() {
        let c = fixture();
        let mut i = Inventory::new(2).unwrap();
        i.set(0, Some(c.stack("stone", 60).unwrap()), &c).unwrap();
        assert!(i
            .insert(c.stack("stone", 10).unwrap(), &c)
            .unwrap()
            .is_none());
        assert_eq!(i.slots()[0].as_ref().unwrap().count, 64);
        assert_eq!(i.slots()[1].as_ref().unwrap().count, 6);
        assert_eq!(i.remove(1, 3).unwrap().unwrap().count, 3);
        let before = i.clone();
        assert!(i
            .insert(c.stack("bucket", 1).unwrap(), &c)
            .unwrap()
            .is_some());
        assert_eq!(i, before);
    }
    #[test]
    fn effective_components_control_merging() {
        let c = fixture();
        let mut base = c.stack("stone", 1).unwrap();
        let mut same = base.clone();
        same.patch
            .insert("minecraft:max_stack_size".into(), Some(Value::from(64)));
        assert!(base.same_kind(&same, &c));
        base.patch
            .insert("minecraft:custom_name".into(), Some(Value::from("Named")));
        assert!(!base.same_kind(&same, &c));
    }
    #[test]
    fn limits_and_durability_follow_components() {
        let c = fixture();
        assert!(c.stack("bucket", 17).is_err());
        let mut t = c.stack("iron_pickaxe", 1).unwrap();
        assert!(!t.damage(&c, 249).unwrap());
        assert!(t.damage(&c, 1).unwrap());
        assert!(c.stack("iron_pickaxe", 2).is_err());
        assert!(c.stack("air", 1).is_err());
    }
}
