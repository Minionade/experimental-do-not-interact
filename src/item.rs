//! Item stacks and the player inventory.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemStack { pub name: String, pub count: u8 }

impl ItemStack {
    pub fn new(name: &str, count: u8) -> Self { Self { name: normalize(name), count } }
}

/// "stone" and "minecraft:stone" are the same item.
pub fn normalize(name: &str) -> String { if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") } }

pub fn max_stack(name: &str) -> u8 {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    if n.ends_with("_pickaxe") || n.ends_with("_axe") || n.ends_with("_shovel") || n.ends_with("_hoe") || n.ends_with("_sword")
        || n.ends_with("_helmet") || n.ends_with("_chestplate") || n.ends_with("_leggings") || n.ends_with("_boots")
        || n == "shears" || n == "bucket" || n.ends_with("_bucket") || n.ends_with("_bed") { return 1; }
    if matches!(n, "ender_pearl" | "snowball" | "egg" | "sign" | "honey_bottle") || n.ends_with("_sign") { return 16; }
    64
}

pub const HOTBAR: usize = 9;
pub const SLOTS: usize = 36;

/// 36 slots: 0..9 are the hotbar, 9..36 the main inventory (vanilla ordering).
#[derive(Clone, Debug)]
pub struct Inventory { pub slots: Vec<Option<ItemStack>>, pub selected: usize }

impl Default for Inventory { fn default() -> Self { Self { slots: vec![None; SLOTS], selected: 0 } } }

impl Inventory {
    pub fn new() -> Self { Self::default() }

    /// Adds a stack: tops up existing stacks first, then fills empty slots (hotbar before main). Returns the leftover count.
    pub fn add(&mut self, stack: &ItemStack) -> u8 {
        let (mut left, max) = (stack.count as u32, max_stack(&stack.name) as u32);
        for s in self.slots.iter_mut().flatten() {
            if left == 0 { break; }
            if s.name == stack.name && (s.count as u32) < max {
                let n = (max - s.count as u32).min(left);
                s.count += n as u8; left -= n;
            }
        }
        for slot in self.slots.iter_mut() {
            if left == 0 { break; }
            if slot.is_none() {
                let n = max.min(left);
                *slot = Some(ItemStack { name: stack.name.clone(), count: n as u8 }); left -= n;
            }
        }
        left as u8
    }

    pub fn count(&self, name: &str) -> u32 {
        let name = normalize(name);
        self.slots.iter().flatten().filter(|s| s.name == name).map(|s| s.count as u32).sum()
    }

    /// Removes up to `n` of an item across stacks (last slots first, like picking from the main inventory). True if all were removed.
    pub fn remove(&mut self, name: &str, mut n: u32) -> bool {
        if self.count(name) < n { return false; }
        let name = normalize(name);
        for slot in self.slots.iter_mut().rev() {
            if n == 0 { break; }
            if let Some(s) = slot { if s.name == name {
                let t = (s.count as u32).min(n);
                s.count -= t as u8; n -= t;
                if s.count == 0 { *slot = None; }
            } }
        }
        true
    }

    pub fn selected_stack(&self) -> Option<&ItemStack> { self.slots[self.selected].as_ref() }

    /// Takes one item from the selected hotbar slot.
    pub fn take_selected(&mut self) -> Option<ItemStack> {
        let slot = &mut self.slots[self.selected];
        let s = slot.as_mut()?;
        s.count -= 1;
        let out = ItemStack { name: s.name.clone(), count: 1 };
        if s.count == 0 { *slot = None; }
        Some(out)
    }

    /// Removes and returns every stack (for dropping on death).
    pub fn drain(&mut self) -> Vec<ItemStack> { self.slots.iter_mut().filter_map(|s| s.take()).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacks_fill_existing_then_empty_slots_hotbar_first() {
        let mut inv = Inventory::new();
        assert_eq!(inv.add(&ItemStack::new("stone", 60)), 0);
        assert_eq!(inv.add(&ItemStack::new("minecraft:stone", 10)), 0);
        assert_eq!(inv.slots[0], Some(ItemStack::new("stone", 64)));
        assert_eq!(inv.slots[1], Some(ItemStack::new("stone", 6)));
        assert_eq!(inv.count("stone"), 70);
        inv.add(&ItemStack::new("dirt", 1));
        assert_eq!(inv.slots[2].as_ref().unwrap().name, "minecraft:dirt");
    }

    #[test]
    fn full_inventory_returns_leftovers_and_tools_do_not_stack() {
        let mut inv = Inventory::new();
        for _ in 0..SLOTS { assert_eq!(inv.add(&ItemStack::new("iron_pickaxe", 1)), 0); }
        assert_eq!(inv.add(&ItemStack::new("iron_pickaxe", 1)), 1);
        assert!(inv.slots.iter().flatten().all(|s| s.count == 1));
        let mut inv = Inventory::new();
        for i in 0..SLOTS { inv.slots[i] = Some(ItemStack::new("stone", 64)); }
        assert_eq!(inv.add(&ItemStack::new("stone", 5)), 5);
        inv.slots[17] = Some(ItemStack::new("stone", 62));
        assert_eq!(inv.add(&ItemStack::new("stone", 5)), 3, "tops up the partial stack, returns the rest");
        assert_eq!(max_stack("minecraft:ender_pearl"), 16);
        assert_eq!(max_stack("cobblestone"), 64);
    }

    #[test]
    fn remove_take_and_drain() {
        let mut inv = Inventory::new();
        inv.add(&ItemStack::new("oak_planks", 70));
        assert!(!inv.remove("oak_planks", 71));
        assert_eq!(inv.count("oak_planks"), 70, "failed removal changes nothing");
        assert!(inv.remove("oak_planks", 66));
        assert_eq!(inv.count("oak_planks"), 4);
        inv.selected = 0;
        let one = inv.take_selected().unwrap();
        assert_eq!(one.count, 1);
        assert_eq!(inv.count("oak_planks"), 3);
        inv.add(&ItemStack::new("dirt", 2));
        let all = inv.drain();
        assert_eq!(all.iter().map(|s| s.count as u32).sum::<u32>(), 5);
        assert_eq!(inv.count("dirt"), 0);
        assert!(inv.take_selected().is_none());
    }
}
