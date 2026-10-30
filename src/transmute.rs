//! Content-preserving crafting for dyed containers and cloned maps.
use crate::{
    crafting::{parse_result, CraftingGrid, Ingredient, ItemTags},
    items::{ItemCatalog, ItemStack},
    vanilla::ResourceId,
};
use serde_json::Value;
pub struct TransmuteRecipe {
    input: Ingredient,
    material: Ingredient,
    min: usize,
    max: usize,
    add_material_count: bool,
    result: Value,
    result_id: Option<String>,
}
impl TransmuteRecipe {
    pub(crate) fn parse(
        value: &Value,
        tags: &ItemTags,
        catalog: &ItemCatalog,
    ) -> Result<Self, String> {
        let (min, max) = match value.get("material_count") {
            None => (1, 1),
            Some(Value::Number(n)) => {
                let n = n.as_u64().ok_or("invalid material count")?;
                (n, n)
            }
            Some(Value::Object(map)) => (
                map.get("min")
                    .map(|v| v.as_u64().ok_or("invalid minimum"))
                    .transpose()?
                    .unwrap_or(1),
                map.get("max")
                    .map(|v| v.as_u64().ok_or("invalid maximum"))
                    .transpose()?
                    .unwrap_or(8),
            ),
            _ => return Err("invalid material count bounds".into()),
        };
        if min < 1 || max > 8 || min > max {
            return Err("material count must fit a crafting grid".into());
        }
        let result = value
            .get("result")
            .filter(|v| v.is_object())
            .ok_or("missing transmute result")?
            .clone();
        let result_id = result
            .get("id")
            .map(|v| ResourceId::parse(v.as_str().ok_or("invalid result ID")?).map(|id| id.key()))
            .transpose()?;
        if result_id.is_some() {
            parse_result(&result, catalog)?;
        }
        Ok(Self {
            input: Ingredient::parse(&value["input"], tags, catalog)?,
            material: Ingredient::parse(&value["material"], tags, catalog)?,
            min: min as usize,
            max: max as usize,
            result,
            result_id,
            add_material_count: value
                .get("add_material_count_to_result")
                .map(|v| v.as_bool().ok_or("invalid add_material_count_to_result"))
                .transpose()?
                .unwrap_or(false),
        })
    }
    pub fn preview(
        &self,
        grid: &CraftingGrid,
        catalog: &ItemCatalog,
    ) -> Result<Option<(Vec<usize>, ItemStack)>, String> {
        grid.validate(catalog)?;
        let mut base = None;
        let mut materials = Vec::new();
        for (i, stack) in grid.slots.iter().enumerate() {
            let Some(stack) = stack else {
                continue;
            };
            if self.input.accepts(stack) && self.result_id.as_ref() != Some(&stack.item) {
                if base.replace(i).is_some() {
                    return Ok(None);
                }
            } else if self.material.accepts(stack) {
                materials.push(i);
            } else {
                return Ok(None);
            }
        }
        let Some(base) = base else {
            return Ok(None);
        };
        if !(self.min..=self.max).contains(&materials.len()) {
            return Ok(None);
        }
        let source = grid.slots[base].as_ref().unwrap();
        let mut result = self.result.clone();
        if self.result_id.is_none() {
            result["id"] = Value::String(source.item.clone());
        }
        let mut template = parse_result(&result, catalog)?;
        if self.add_material_count {
            template.count = template
                .count
                .checked_add(materials.len() as u32)
                .ok_or("result count overflow")?;
        }
        let result = source.transmute(template, catalog)?;
        materials.push(base);
        Ok(Some((materials, result)))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn counts_material_slots_and_rejects_two_bases() {
        let c = crate::items::fixture();
        let recipe = TransmuteRecipe::parse(&json!({"input":"minecraft:stone","material":"minecraft:stick","material_count":{"min":1,"max":8},"add_material_count_to_result":true,"result":{}}),&ItemTags::default(),&c).unwrap();
        let mut grid = CraftingGrid {
            width: 2,
            height: 2,
            slots: vec![
                Some(c.stack("stone", 3).unwrap()),
                Some(c.stack("stick", 64).unwrap()),
                Some(c.stack("stick", 12).unwrap()),
                None,
            ],
        };
        grid.slots[0].as_mut().unwrap().patch.insert(
            "minecraft:custom_data".into(),
            Some(json!({"contents":[1,2,3]})),
        );
        let (slots, result) = recipe.preview(&grid, &c).unwrap().unwrap();
        assert_eq!(slots.len(), 3);
        assert_eq!(result.count, 3);
        assert_eq!(result.patch, grid.slots[0].as_ref().unwrap().patch);
        grid.slots[3] = Some(c.stack("stone", 1).unwrap());
        assert!(recipe.preview(&grid, &c).unwrap().is_none());
    }
    #[test]
    fn same_result_item_and_extra_materials_are_rejected() {
        let c = crate::items::fixture();
        let recipe = TransmuteRecipe::parse(&json!({"input":["minecraft:stone","minecraft:oak_planks"],"material":"minecraft:stick","result":{"id":"minecraft:oak_planks"}}),&ItemTags::default(),&c).unwrap();
        let mut grid = CraftingGrid {
            width: 2,
            height: 2,
            slots: vec![
                Some(c.stack("oak_planks", 1).unwrap()),
                Some(c.stack("stick", 1).unwrap()),
                None,
                None,
            ],
        };
        assert!(recipe.preview(&grid, &c).unwrap().is_none());
        grid.slots[0] = Some(c.stack("stone", 1).unwrap());
        assert!(recipe.preview(&grid, &c).unwrap().is_some());
        grid.slots[2] = Some(c.stack("stick", 1).unwrap());
        assert!(recipe.preview(&grid, &c).unwrap().is_none());
    }
}
