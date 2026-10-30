//! Ordinary vanilla recipes with exact tags and atomic grid/inventory mutations.
use crate::{
    items::{Inventory, ItemCatalog, ItemStack},
    vanilla::{ClientAssets, ResourceId},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Default)]
pub struct ItemTags {
    tags: BTreeMap<String, BTreeSet<String>>,
}
impl ItemTags {
    pub fn load(assets: &mut ClientAssets, catalog: &ItemCatalog) -> Result<Self, String> {
        let ids: Vec<_> = assets
            .resource_paths()
            .filter_map(|p| {
                p.strip_prefix("data/minecraft/tags/item/")
                    .and_then(|p| p.strip_suffix(".json"))
                    .map(|p| format!("minecraft:{p}"))
            })
            .collect();
        let mut raw = BTreeMap::new();
        for id in ids {
            raw.insert(
                id.clone(),
                assets.data_json("tags/item", &ResourceId::parse(&id)?)?,
            );
        }
        let mut result = Self::default();
        for id in raw.keys() {
            resolve_tag(id, &raw, catalog, &mut result.tags, &mut BTreeSet::new())?;
        }
        Ok(result)
    }
    pub fn len(&self) -> usize {
        self.tags.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }
}
fn resolve_tag(
    id: &str,
    raw: &BTreeMap<String, Value>,
    catalog: &ItemCatalog,
    cache: &mut BTreeMap<String, BTreeSet<String>>,
    visiting: &mut BTreeSet<String>,
) -> Result<BTreeSet<String>, String> {
    if let Some(tag) = cache.get(id) {
        return Ok(tag.clone());
    }
    if visiting.len() >= 64 || !visiting.insert(id.into()) {
        return Err(format!("item tag cycle/depth at {id}"));
    }
    let mut values = BTreeSet::new();
    let value = raw.get(id).ok_or_else(|| format!("missing tag {id}"))?;
    for entry in value["values"]
        .as_array()
        .ok_or("tag values must be array")?
    {
        let (name, required) = if let Some(name) = entry.as_str() {
            (name, true)
        } else {
            (
                entry["id"].as_str().ok_or("tag entry needs ID")?,
                entry
                    .get("required")
                    .map(|v| v.as_bool().ok_or("required must be boolean"))
                    .transpose()?
                    .unwrap_or(true),
            )
        };
        if let Some(tag) = name.strip_prefix('#') {
            let tag = ResourceId::parse(tag)?.key();
            if !raw.contains_key(&tag) && !required {
                continue;
            }
            values.extend(resolve_tag(&tag, raw, catalog, cache, visiting)?);
        } else {
            let name = ResourceId::parse(name)?.key();
            if catalog.get(&name).is_none() {
                if required {
                    return Err(format!("unknown tag item {name}"));
                }
                continue;
            }
            values.insert(name);
        }
    }
    visiting.remove(id);
    cache.insert(id.into(), values.clone());
    Ok(values)
}
#[derive(Clone)]
struct Ingredient(BTreeSet<String>);
impl Ingredient {
    fn parse(value: &Value, tags: &ItemTags, catalog: &ItemCatalog) -> Result<Self, String> {
        let mut values = BTreeSet::new();
        if let Some(array) = value.as_array() {
            if array.is_empty() {
                return Err("empty ingredient alternatives".into());
            }
            for v in array {
                values.extend(Self::parse(v, tags, catalog)?.0);
            }
        } else {
            let id = value
                .as_str()
                .ok_or("ingredient must be ID, tag or alternatives")?;
            if let Some(tag) = id.strip_prefix('#') {
                let tag = ResourceId::parse(tag)?.key();
                values.extend(
                    tags.tags
                        .get(&tag)
                        .ok_or_else(|| format!("unknown ingredient tag {tag}"))?
                        .clone(),
                );
            } else {
                let id = ResourceId::parse(id)?.key();
                if catalog.get(&id).is_none() {
                    return Err(format!("unknown ingredient {id}"));
                }
                values.insert(id);
            }
        }
        Ok(Self(values))
    }
    fn accepts(&self, stack: &ItemStack) -> bool {
        self.0.contains(&stack.item)
    }
}
enum Pattern {
    Shaped {
        width: usize,
        height: usize,
        cells: Vec<Option<Ingredient>>,
    },
    Shapeless(Vec<Ingredient>),
}
pub struct CraftingRecipe {
    pattern: Pattern,
    pub result: ItemStack,
}
pub struct CookingRecipe {
    ingredient: Ingredient,
    pub result: ItemStack,
    pub ticks: u32,
    pub experience: f64,
    pub kind: String,
}
impl CookingRecipe {
    pub fn accepts(&self, stack: &ItemStack) -> bool {
        self.ingredient.accepts(stack)
    }
}
pub struct RecipeBook {
    pub crafting: BTreeMap<String, CraftingRecipe>,
    pub cooking: BTreeMap<String, CookingRecipe>,
    pub unsupported: BTreeMap<String, String>,
    pub raw: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CraftingGrid {
    pub width: usize,
    pub height: usize,
    pub slots: Vec<Option<ItemStack>>,
}
impl CraftingGrid {
    pub fn validate(&self, catalog: &ItemCatalog) -> Result<(), String> {
        if !(1..=3).contains(&self.width)
            || !(1..=3).contains(&self.height)
            || self.slots.len() != self.width * self.height
        {
            return Err("invalid crafting dimensions".into());
        }
        for stack in self.slots.iter().flatten() {
            stack.validate(catalog)?;
        }
        Ok(())
    }
}
impl RecipeBook {
    pub fn load(
        assets: &mut ClientAssets,
        tags: &ItemTags,
        catalog: &ItemCatalog,
    ) -> Result<Self, String> {
        let ids: Vec<_> = assets
            .resource_paths()
            .filter_map(|p| {
                p.strip_prefix("data/minecraft/recipe/")
                    .and_then(|p| p.strip_suffix(".json"))
                    .map(|p| format!("minecraft:{p}"))
            })
            .collect();
        let mut book = Self {
            crafting: BTreeMap::new(),
            cooking: BTreeMap::new(),
            unsupported: BTreeMap::new(),
            raw: BTreeMap::new(),
        };
        for id in ids {
            let value = assets.data_json("recipe", &ResourceId::parse(&id)?)?;
            let kind = value["type"]
                .as_str()
                .ok_or("recipe needs type")?
                .to_string();
            match kind.as_str() {
                "minecraft:crafting_shaped" | "minecraft:crafting_shapeless" => {
                    book.crafting.insert(
                        id.clone(),
                        CraftingRecipe::parse(&value, tags, catalog)
                            .map_err(|e| format!("{id}: {e}"))?,
                    );
                }
                "minecraft:smelting"
                | "minecraft:blasting"
                | "minecraft:smoking"
                | "minecraft:campfire_cooking" => {
                    let ticks = value
                        .get("cookingtime")
                        .map(|v| {
                            v.as_u64()
                                .and_then(|n| u32::try_from(n).ok())
                                .filter(|n| *n > 0)
                                .ok_or("invalid cooking time")
                        })
                        .transpose()?
                        .unwrap_or(match kind.as_str() {
                            "minecraft:blasting" | "minecraft:smoking" => 100,
                            "minecraft:campfire_cooking" => 600,
                            _ => 200,
                        });
                    let experience = value
                        .get("experience")
                        .map(|v| {
                            v.as_f64()
                                .filter(|n| n.is_finite() && *n >= 0.)
                                .ok_or("invalid experience")
                        })
                        .transpose()?
                        .unwrap_or(0.);
                    book.cooking.insert(
                        id.clone(),
                        CookingRecipe {
                            ingredient: Ingredient::parse(&value["ingredient"], tags, catalog)?,
                            result: parse_result(&value["result"], catalog)?,
                            ticks,
                            experience,
                            kind,
                        },
                    );
                }
                _ => {
                    book.unsupported.insert(id.clone(), kind);
                }
            }
            book.raw.insert(id, value);
        }
        Ok(book)
    }
    pub fn find_crafting(
        &self,
        grid: &CraftingGrid,
        catalog: &ItemCatalog,
    ) -> Result<Option<&str>, String> {
        grid.validate(catalog)?;
        Ok(self
            .crafting
            .iter()
            .find(|(_, r)| r.match_slots(grid).is_some())
            .map(|(id, _)| id.as_str()))
    }
    /// Full output and all remainders must fit, otherwise nothing is consumed.
    pub fn craft_into(
        &self,
        id: &str,
        grid: &mut CraftingGrid,
        inventory: &mut Inventory,
        catalog: &ItemCatalog,
    ) -> Result<bool, String> {
        grid.validate(catalog)?;
        inventory.validate(catalog)?;
        let id = ResourceId::parse(id)?.key();
        let recipe = self
            .crafting
            .get(&id)
            .ok_or_else(|| format!("unsupported/unknown recipe {id}"))?;
        let Some(slots) = recipe.match_slots(grid) else {
            return Ok(false);
        };
        let mut g = grid.clone();
        let mut inv = inventory.clone();
        if inv.insert(recipe.result.clone(), catalog)?.is_some() {
            return Ok(false);
        }
        for index in slots {
            let stack = g.slots[index].as_mut().unwrap();
            let remainder = crafting_remainder(&stack.item)
                .map(|id| catalog.stack(id, 1))
                .transpose()?;
            stack.count -= 1;
            if stack.count == 0 {
                g.slots[index] = None;
            }
            if let Some(remainder) = remainder {
                if g.slots[index].is_none() {
                    g.slots[index] = Some(remainder);
                } else if inv.insert(remainder, catalog)?.is_some() {
                    return Ok(false);
                }
            }
        }
        *grid = g;
        *inventory = inv;
        Ok(true)
    }
}
fn parse_result(value: &Value, catalog: &ItemCatalog) -> Result<ItemStack, String> {
    let name = value["id"].as_str().ok_or("result requires item ID")?;
    let count = value
        .get("count")
        .map(|v| {
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("invalid result count")
        })
        .transpose()?
        .unwrap_or(1);
    let mut stack = catalog.stack(name, count)?;
    if let Some(c) = value.get("components") {
        for (key, value) in c.as_object().ok_or("components must be object")? {
            if let Some(key) = key.strip_prefix('!') {
                stack.patch.insert(ResourceId::parse(key)?.key(), None);
            } else {
                stack
                    .patch
                    .insert(ResourceId::parse(key)?.key(), Some(value.clone()));
            }
        }
    }
    stack.validate(catalog)?;
    Ok(stack)
}
pub(crate) fn crafting_remainder(item: &str) -> Option<&'static str> {
    match item {
        "minecraft:milk_bucket" | "minecraft:water_bucket" | "minecraft:lava_bucket" => {
            Some("minecraft:bucket")
        }
        "minecraft:honey_bottle" | "minecraft:dragon_breath" => Some("minecraft:glass_bottle"),
        _ => None,
    }
}
impl CraftingRecipe {
    fn parse(value: &Value, tags: &ItemTags, catalog: &ItemCatalog) -> Result<Self, String> {
        let result = parse_result(&value["result"], catalog)?;
        let pattern = if value["type"] == "minecraft:crafting_shapeless" {
            let a = value["ingredients"]
                .as_array()
                .ok_or("missing ingredients")?;
            if !(1..=9).contains(&a.len()) {
                return Err("invalid ingredient count".into());
            }
            Pattern::Shapeless(
                a.iter()
                    .map(|v| Ingredient::parse(v, tags, catalog))
                    .collect::<Result<_, _>>()?,
            )
        } else {
            let rows: Vec<String> =
                serde_json::from_value(value["pattern"].clone()).map_err(|e| e.to_string())?;
            if rows.is_empty()
                || rows.len() > 3
                || rows
                    .iter()
                    .any(|r| !r.is_ascii() || r.len() != rows[0].len())
                || !(1..=3).contains(&rows[0].len())
            {
                return Err("invalid pattern dimensions".into());
            }
            let mut bounds = (3, 3, 0, 0);
            for (y, row) in rows.iter().enumerate() {
                for (x, c) in row.bytes().enumerate() {
                    if c != b' ' {
                        bounds.0 = bounds.0.min(x);
                        bounds.1 = bounds.1.min(y);
                        bounds.2 = bounds.2.max(x);
                        bounds.3 = bounds.3.max(y);
                    }
                }
            }
            if bounds.0 > bounds.2 {
                return Err("empty pattern".into());
            }
            let mut key = BTreeMap::new();
            for (name, v) in value["key"].as_object().ok_or("missing pattern key")? {
                if name.len() != 1 || !name.is_ascii() || name == " " {
                    return Err("invalid pattern symbol".into());
                }
                key.insert(name.as_bytes()[0], Ingredient::parse(v, tags, catalog)?);
            }
            let mut cells = Vec::new();
            for row in rows.iter().take(bounds.3 + 1).skip(bounds.1) {
                for c in row.bytes().skip(bounds.0).take(bounds.2 - bounds.0 + 1) {
                    cells.push(if c == b' ' {
                        None
                    } else {
                        Some(key.get(&c).ok_or("undefined pattern symbol")?.clone())
                    });
                }
            }
            Pattern::Shaped {
                width: bounds.2 - bounds.0 + 1,
                height: bounds.3 - bounds.1 + 1,
                cells,
            }
        };
        Ok(Self { pattern, result })
    }
    fn match_slots(&self, grid: &CraftingGrid) -> Option<Vec<usize>> {
        match &self.pattern {
            Pattern::Shaped {
                width,
                height,
                cells,
            } => {
                if *width > grid.width || *height > grid.height {
                    return None;
                }
                for dy in 0..=grid.height - height {
                    for dx in 0..=grid.width - width {
                        for mirror in [false, true] {
                            let mut matched = true;
                            let mut used = Vec::new();
                            for y in 0..grid.height {
                                for x in 0..grid.width {
                                    let slot = y * grid.width + x;
                                    let ingredient = if x >= dx
                                        && x < dx + width
                                        && y >= dy
                                        && y < dy + height
                                    {
                                        let sx = if mirror { width - 1 - (x - dx) } else { x - dx };
                                        cells[(y - dy) * width + sx].as_ref()
                                    } else {
                                        None
                                    };
                                    match (ingredient, &grid.slots[slot]) {
                                        (None, None) => {}
                                        (Some(i), Some(s)) if i.accepts(s) => used.push(slot),
                                        _ => matched = false,
                                    }
                                }
                            }
                            if matched {
                                return Some(used);
                            }
                        }
                    }
                }
                None
            }
            Pattern::Shapeless(ingredients) => {
                let occupied: Vec<_> = grid
                    .slots
                    .iter()
                    .enumerate()
                    .filter_map(|(i, s)| s.as_ref().map(|s| (i, s)))
                    .collect();
                if occupied.len() != ingredients.len() {
                    return None;
                }
                let masks: Vec<u16> = ingredients
                    .iter()
                    .map(|ingredient| {
                        occupied.iter().enumerate().fold(0, |m, (i, (_, s))| {
                            m | if ingredient.accepts(s) { 1 << i } else { 0 }
                        })
                    })
                    .collect();
                let mut possible = BTreeSet::from([0u16]);
                for mask in masks {
                    let mut next = BTreeSet::new();
                    for used in possible {
                        for i in 0..occupied.len() {
                            let bit = 1 << i;
                            if mask & bit != 0 && used & bit == 0 {
                                next.insert(used | bit);
                            }
                        }
                    }
                    possible = next;
                    if possible.is_empty() {
                        return None;
                    }
                }
                Some(occupied.into_iter().map(|(i, _)| i).collect())
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn recipe(v: Value) -> (CraftingRecipe, ItemCatalog) {
        let c = crate::items::fixture();
        (
            CraftingRecipe::parse(&v, &ItemTags::default(), &c).unwrap(),
            c,
        )
    }
    #[test]
    fn shape_offset_mirror_and_extra_item() {
        let (r, c) = recipe(
            json!({"type":"minecraft:crafting_shaped","pattern":["AB"],"key":{"A":"minecraft:iron_ingot","B":"minecraft:stick"},"result":{"id":"minecraft:iron_pickaxe"}}),
        );
        let mut g = CraftingGrid {
            width: 3,
            height: 3,
            slots: vec![None; 9],
        };
        g.slots[4] = Some(c.stack("stick", 1).unwrap());
        g.slots[5] = Some(c.stack("iron_ingot", 1).unwrap());
        assert!(r.match_slots(&g).is_some());
        g.slots[0] = Some(c.stack("stone", 1).unwrap());
        assert!(r.match_slots(&g).is_none());
    }
    #[test]
    fn shapeless_assignment_handles_overlapping_alternatives() {
        let (r, c) = recipe(
            json!({"type":"minecraft:crafting_shapeless","ingredients":[["minecraft:stone","minecraft:stick"],"minecraft:stone"],"result":{"id":"minecraft:oak_planks"}}),
        );
        let g = CraftingGrid {
            width: 2,
            height: 2,
            slots: vec![
                Some(c.stack("stone", 1).unwrap()),
                Some(c.stack("stick", 1).unwrap()),
                None,
                None,
            ],
        };
        assert!(r.match_slots(&g).is_some());
    }
    #[test]
    fn transaction_and_container_remainder() {
        let (r, c) = recipe(
            json!({"type":"minecraft:crafting_shapeless","ingredients":["minecraft:milk_bucket"],"result":{"id":"minecraft:stone"}}),
        );
        let book = RecipeBook {
            crafting: [("minecraft:test".into(), r)].into(),
            cooking: BTreeMap::new(),
            unsupported: BTreeMap::new(),
            raw: BTreeMap::new(),
        };
        let mut g = CraftingGrid {
            width: 1,
            height: 1,
            slots: vec![Some(c.stack("milk_bucket", 1).unwrap())],
        };
        let mut inv = Inventory::new(1).unwrap();
        inv.set(0, Some(c.stack("stick", 64).unwrap()), &c).unwrap();
        let before = g.clone();
        assert!(!book.craft_into("test", &mut g, &mut inv, &c).unwrap());
        assert_eq!(g, before);
        inv.remove(0, 64).unwrap();
        assert!(book.craft_into("test", &mut g, &mut inv, &c).unwrap());
        assert_eq!(g.slots[0].as_ref().unwrap().item, "minecraft:bucket");
    }
    #[test]
    fn tag_cycles_fail() {
        let c = crate::items::fixture();
        let raw = [
            ("minecraft:a".into(), json!({"values":["#minecraft:b"]})),
            ("minecraft:b".into(), json!({"values":["#minecraft:a"]})),
        ]
        .into();
        assert!(resolve_tag(
            "minecraft:a",
            &raw,
            &c,
            &mut BTreeMap::new(),
            &mut BTreeSet::new()
        )
        .is_err());
    }
}
