//! Smithing-transform transactions; trim recipes require a separate implementation.
use crate::{
    crafting::{parse_result, Ingredient, ItemTags},
    items::{Inventory, ItemCatalog, ItemStack},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmithingInput {
    pub template: Option<ItemStack>,
    pub base: Option<ItemStack>,
    pub addition: Option<ItemStack>,
}
impl SmithingInput {
    pub fn validate(&self, catalog: &ItemCatalog) -> Result<(), String> {
        for stack in [&self.template, &self.base, &self.addition]
            .into_iter()
            .flatten()
        {
            stack.validate(catalog)?;
        }
        Ok(())
    }
}
pub struct TransformRecipe {
    template: Option<Ingredient>,
    base: Ingredient,
    addition: Option<Ingredient>,
    pub result: ItemStack,
}
impl TransformRecipe {
    pub(crate) fn parse(
        value: &Value,
        tags: &ItemTags,
        catalog: &ItemCatalog,
    ) -> Result<Self, String> {
        Ok(Self {
            template: value
                .get("template")
                .map(|v| Ingredient::parse(v, tags, catalog))
                .transpose()?,
            base: Ingredient::parse(&value["base"], tags, catalog)?,
            addition: value
                .get("addition")
                .map(|v| Ingredient::parse(v, tags, catalog))
                .transpose()?,
            result: parse_result(&value["result"], catalog)?,
        })
    }
    pub fn matches(&self, input: &SmithingInput) -> bool {
        fn optional(ingredient: &Option<Ingredient>, stack: &Option<ItemStack>) -> bool {
            match (ingredient, stack) {
                (None, None) => true,
                (Some(i), Some(s)) => i.accepts(s),
                _ => false,
            }
        }
        optional(&self.template, &input.template)
            && input.base.as_ref().is_some_and(|s| self.base.accepts(s))
            && optional(&self.addition, &input.addition)
    }
    pub fn preview(
        &self,
        input: &SmithingInput,
        catalog: &ItemCatalog,
    ) -> Result<Option<ItemStack>, String> {
        input.validate(catalog)?;
        if !self.matches(input) {
            return Ok(None);
        }
        let mut result = self.result.clone();
        // Copy the source's explicit changes, not its item's prototype defaults.
        // Recipe-authored component changes take precedence over source changes.
        let base = input.base.as_ref().unwrap();
        let base_defaults = &catalog
            .get(&base.item)
            .ok_or("unknown base item")?
            .components;
        let mut patch: std::collections::BTreeMap<_, _> = base
            .patch
            .iter()
            .filter(|(key, value)| value.as_ref() != base_defaults.get(*key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        patch.extend(result.patch);
        let output_defaults = &catalog
            .get(&result.item)
            .ok_or("unknown result item")?
            .components;
        patch.retain(|key, value| value.as_ref() != output_defaults.get(key));
        result.patch = patch;
        result.validate(catalog)?;
        Ok(Some(result))
    }
    pub fn craft_into(
        &self,
        input: &mut SmithingInput,
        inventory: &mut Inventory,
        catalog: &ItemCatalog,
    ) -> Result<bool, String> {
        inventory.validate(catalog)?;
        let Some(result) = self.preview(input, catalog)? else {
            return Ok(false);
        };
        let mut next = inventory.clone();
        if next.insert(result, catalog)?.is_some() {
            return Ok(false);
        }
        for slot in [&mut input.template, &mut input.base, &mut input.addition] {
            if let Some(stack) = slot {
                stack.count -= 1;
                if stack.count == 0 {
                    *slot = None;
                }
            }
        }
        *inventory = next;
        Ok(true)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn retains_components_and_consumes_all_three_slots_atomically() {
        let c = crate::items::fixture();
        let recipe = TransformRecipe::parse(&json!({"template":"minecraft:stone","base":"minecraft:iron_pickaxe","addition":"minecraft:iron_ingot","result":{"id":"minecraft:iron_pickaxe"}}),&ItemTags::default(),&c).unwrap();
        let mut base = c.stack("iron_pickaxe", 1).unwrap();
        base.patch
            .insert("minecraft:damage".into(), Some(json!(17)));
        base.patch
            .insert("minecraft:custom_name".into(), Some(json!("Keep me")));
        let mut input = SmithingInput {
            template: Some(c.stack("stone", 2).unwrap()),
            base: Some(base.clone()),
            addition: Some(c.stack("iron_ingot", 2).unwrap()),
        };
        let mut inventory = Inventory::new(1).unwrap();
        inventory
            .set(0, Some(c.stack("stick", 64).unwrap()), &c)
            .unwrap();
        let original = input.clone();
        let slots = inventory.clone();
        assert!(!recipe.craft_into(&mut input, &mut inventory, &c).unwrap());
        assert_eq!(input, original);
        assert_eq!(inventory, slots);
        inventory.remove(0, 64).unwrap();
        assert!(recipe.craft_into(&mut input, &mut inventory, &c).unwrap());
        assert!(input.base.is_none());
        assert_eq!(input.template.as_ref().unwrap().count, 1);
        assert_eq!(input.addition.as_ref().unwrap().count, 1);
        assert_eq!(inventory.slots()[0].as_ref().unwrap().patch, base.patch);
    }
    #[test]
    fn redundant_source_defaults_do_not_replace_target_defaults() {
        let c = crate::items::fixture();
        let recipe = TransformRecipe::parse(
            &json!({"base":"minecraft:iron_pickaxe","result":{"id":"minecraft:stone"}}),
            &ItemTags::default(),
            &c,
        )
        .unwrap();
        let mut base = c.stack("iron_pickaxe", 1).unwrap();
        base.patch
            .insert("minecraft:max_damage".into(), Some(json!(250)));
        base.patch.insert("minecraft:damage".into(), Some(json!(0)));
        base.patch.insert("minecraft:repair_cost".into(), None);
        let input = SmithingInput {
            template: None,
            base: Some(base),
            addition: None,
        };
        let output = recipe.preview(&input, &c).unwrap().unwrap();
        assert!(output.patch.is_empty());
        assert_eq!(output.max_count(&c).unwrap(), 64);
    }
    #[test]
    fn recipe_components_override_source_and_wrong_inputs_do_not_match() {
        let c = crate::items::fixture();
        let recipe = TransformRecipe::parse(&json!({"base":"minecraft:iron_pickaxe","result":{"id":"minecraft:iron_pickaxe","components":{"minecraft:damage":7}}}),&ItemTags::default(),&c).unwrap();
        let mut base = c.stack("iron_pickaxe", 1).unwrap();
        base.patch
            .insert("minecraft:damage".into(), Some(json!(17)));
        let mut input = SmithingInput {
            template: None,
            base: Some(base),
            addition: None,
        };
        assert_eq!(
            recipe
                .preview(&input, &c)
                .unwrap()
                .unwrap()
                .component(&c, "minecraft:damage"),
            Some(&json!(7))
        );
        input.addition = Some(c.stack("stick", 1).unwrap());
        assert!(recipe.preview(&input, &c).unwrap().is_none());
    }
}
