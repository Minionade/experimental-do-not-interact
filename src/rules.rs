//! Block hardness, tool rules, break times and drops. Values follow the publicly documented survival mechanics.
use crate::item::ItemStack;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind { Hand, Pickaxe, Axe, Shovel, Hoe, Sword, Shears }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tool {
    pub kind: ToolKind,
    /// Mining level: wood/gold 0, stone 1, iron 2, diamond/netherite 3. None for bare hands.
    pub level: Option<u8>,
    pub speed: f32,
}

pub const HAND: Tool = Tool { kind: ToolKind::Hand, level: None, speed: 1.0 };

/// Parses an item name such as "minecraft:iron_pickaxe".
pub fn tool_of(item: &str) -> Tool {
    let n = item.strip_prefix("minecraft:").unwrap_or(item);
    if n == "shears" { return Tool { kind: ToolKind::Shears, level: Some(0), speed: 5.0 }; }
    let Some((tier, kind)) = n.split_once('_') else { return HAND };
    let kind = match kind { "pickaxe" => ToolKind::Pickaxe, "axe" => ToolKind::Axe, "shovel" => ToolKind::Shovel, "hoe" => ToolKind::Hoe, "sword" => ToolKind::Sword, _ => return HAND };
    let (level, speed) = match tier { "wooden" => (0, 2.0), "stone" => (1, 4.0), "iron" => (2, 6.0), "diamond" => (3, 8.0), "netherite" => (3, 9.0), "golden" => (0, 12.0), _ => return HAND };
    Tool { kind, level: Some(level), speed }
}

#[derive(Clone, Copy, Debug)]
pub struct BlockRule {
    pub hardness: f32,
    pub tool: ToolKind,
    /// Minimum mining level to get drops; None means any tool (or hand) works.
    pub min_level: Option<u8>,
}

fn rule(hardness: f32, tool: ToolKind, min_level: Option<u8>) -> Option<BlockRule> { Some(BlockRule { hardness, tool, min_level }) }

/// None means the block cannot be broken in survival (bedrock, fluids, air...).
pub fn block_rule(name: &str) -> Option<BlockRule> {
    use ToolKind::*;
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    match n {
        "bedrock" | "barrier" | "air" | "cave_air" | "void_air" | "water" | "lava" | "end_portal_frame" | "command_block" => None,
        "stone" | "granite" | "diorite" | "andesite" | "tuff" => rule(1.5, Pickaxe, Some(0)),
        "cobblestone" | "mossy_cobblestone" | "bricks" | "stone_bricks" => rule(2.0, Pickaxe, Some(0)),
        "deepslate" => rule(3.0, Pickaxe, Some(0)),
        "sandstone" | "red_sandstone" => rule(0.8, Pickaxe, Some(0)),
        "netherrack" => rule(0.4, Pickaxe, Some(0)),
        "obsidian" | "crying_obsidian" => rule(50.0, Pickaxe, Some(3)),
        "coal_ore" | "copper_ore" => rule(3.0, Pickaxe, Some(if n == "coal_ore" { 0 } else { 1 })),
        "iron_ore" | "lapis_ore" => rule(3.0, Pickaxe, Some(1)),
        "gold_ore" | "diamond_ore" | "redstone_ore" | "emerald_ore" => rule(3.0, Pickaxe, Some(2)),
        "deepslate_coal_ore" => rule(4.5, Pickaxe, Some(0)),
        "deepslate_iron_ore" | "deepslate_copper_ore" | "deepslate_lapis_ore" => rule(4.5, Pickaxe, Some(1)),
        "deepslate_gold_ore" | "deepslate_diamond_ore" | "deepslate_redstone_ore" | "deepslate_emerald_ore" => rule(4.5, Pickaxe, Some(2)),
        "furnace" | "blast_furnace" | "smoker" => rule(3.5, Pickaxe, Some(0)),
        "ice" | "packed_ice" => rule(0.5, Pickaxe, None),
        "dirt" | "coarse_dirt" | "podzol" | "rooted_dirt" => rule(0.5, Shovel, None),
        "grass_block" | "mycelium" | "gravel" | "clay" | "dirt_path" | "farmland" => rule(0.6, Shovel, None),
        "sand" | "red_sand" | "soul_sand" => rule(0.5, Shovel, None),
        "snow_block" | "snow" => rule(0.2, Shovel, Some(0)),
        "crafting_table" | "chest" | "bookshelf" => rule(2.5, Axe, None),
        "glass" | "glowstone" | "sea_lantern" => rule(0.3, Hand, None),
        "torch" | "wall_torch" | "short_grass" | "tall_grass" | "fern" | "dandelion" | "poppy" | "vine" => rule(0.0, Hand, None),
        _ if n.ends_with("_log") || n.ends_with("_wood") || n.ends_with("_planks") || n.ends_with("_fence") || n.ends_with("_door") => rule(2.0, Axe, None),
        _ if n.ends_with("_leaves") => rule(0.2, Hoe, None),
        _ if n.ends_with("_wool") => rule(0.8, Shears, None),
        _ if n.ends_with("_ore") => rule(3.0, Pickaxe, Some(2)),
        _ if n.ends_with("_stairs") || n.ends_with("_slab") => rule(2.0, Pickaxe, Some(0)),
        _ => rule(1.0, Hand, None),
    }
}

/// True if breaking with `tool` yields drops.
pub fn can_harvest(r: &BlockRule, tool: &Tool) -> bool {
    match r.min_level {
        None => true,
        Some(l) => tool.kind == r.tool && tool.level.is_some_and(|tl| tl >= l)
            || r.tool == ToolKind::Shears && tool.kind == ToolKind::Shears,
    }
}

/// Fraction of a block broken per tick (>= 1 means instant). Documented formula:
/// speed / hardness / (30 if harvestable else 100), divided by 5 with the head in water and by 5 while airborne.
pub fn progress_per_tick(r: &BlockRule, tool: &Tool, head_in_water: bool, on_ground: bool) -> f32 {
    if r.hardness <= 0.0 { return 1.0; }
    let speed = if r.tool != ToolKind::Hand && tool.kind == r.tool { tool.speed } else { 1.0 };
    let mut p = speed / r.hardness / if can_harvest(r, tool) { 30.0 } else { 100.0 };
    if head_in_water { p /= 5.0; }
    if !on_ground { p /= 5.0; }
    p
}

/// Ticks needed to break a block under constant conditions (0 = instant).
pub fn break_ticks(r: &BlockRule, tool: &Tool, head_in_water: bool, on_ground: bool) -> u32 {
    let p = progress_per_tick(r, tool, head_in_water, on_ground);
    if p >= 1.0 { 0 } else { (1.0 / p).ceil() as u32 }
}

/// Items dropped by breaking `name` with `tool`. `roll` is a uniform value in [0, 1) for chance drops.
pub fn drops(name: &str, tool: &Tool, roll: f32) -> Vec<ItemStack> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let Some(r) = block_rule(name) else { return vec![] };
    if !can_harvest(&r, tool) { return vec![]; }
    let one = |i: &str| vec![ItemStack::new(i, 1)];
    match n {
        "stone" => one("cobblestone"),
        "deepslate" => one("cobbled_deepslate"),
        "grass_block" | "podzol" | "mycelium" | "dirt_path" | "farmland" => one("dirt"),
        "coal_ore" | "deepslate_coal_ore" => one("coal"),
        "diamond_ore" | "deepslate_diamond_ore" => one("diamond"),
        "iron_ore" | "deepslate_iron_ore" => one("raw_iron"),
        "gold_ore" | "deepslate_gold_ore" => one("raw_gold"),
        "glowstone" => vec![ItemStack::new("glowstone_dust", 2 + (roll * 3.0) as u8)],
        "gravel" => if roll < 0.1 { one("flint") } else { one("gravel") },
        "glass" | "ice" | "sea_lantern" | "tall_grass" | "vine" => vec![],
        "short_grass" => if roll < 0.125 { one("wheat_seeds") } else { vec![] },
        "snow" | "snow_block" => vec![ItemStack::new("snowball", if n == "snow_block" { 4 } else { 1 })],
        "oak_leaves" | "birch_leaves" | "spruce_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves" => {
            if roll < 0.05 { one(&n.replace("_leaves", "_sapling")) } else if roll < 0.07 { one("stick") } else { vec![] }
        }
        _ if n.ends_with("_wood") => one(&n.replace("_wood", "_log")),
        _ => one(n),
    }
}

/// Whether an item is an ordinary placeable block (as opposed to a tool, material or food).
pub fn is_block_item(item: &str) -> bool {
    let n = item.strip_prefix("minecraft:").unwrap_or(item);
    if tool_of(item).kind != ToolKind::Hand || n == "shears" { return false; }
    !matches!(n, "stick" | "coal" | "diamond" | "raw_iron" | "raw_gold" | "iron_ingot" | "gold_ingot" | "flint" | "apple" | "bread"
        | "glowstone_dust" | "string" | "wheat_seeds" | "snowball" | "charcoal" | "emerald" | "lapis_lazuli" | "redstone" | "bone")
        && !n.ends_with("_bucket") && n != "bucket"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticks(block: &str, tool: &str) -> u32 { break_ticks(&block_rule(block).unwrap(), &tool_of(tool), false, true) }

    #[test]
    fn break_times_match_documented_values() {
        // Seconds at 20 ticks/s, from the survival mining tables.
        assert_eq!(ticks("stone", "hand"), 150, "7.5 s by hand");
        assert_eq!(ticks("stone", "wooden_pickaxe"), 23, "1.15 s");
        assert_eq!(ticks("stone", "iron_pickaxe"), 8, "0.4 s");
        assert_eq!(ticks("dirt", "hand"), 15, "0.75 s");
        assert_eq!(ticks("dirt", "iron_shovel"), 3);
        assert_eq!(ticks("oak_log", "hand"), 60, "3 s");
        assert_eq!(ticks("oak_log", "iron_axe"), 10, "0.5 s");
        assert_eq!(ticks("obsidian", "diamond_pickaxe"), 188, "9.4 s");
        assert_eq!(ticks("obsidian", "iron_pickaxe"), 834, "can't harvest: /100 divisor, 41.7 s, no drop");
        assert_eq!(ticks("torch", "hand"), 0, "instant");
        assert_eq!(ticks("diamond_ore", "iron_pickaxe"), 15);
        assert!(block_rule("bedrock").is_none() && block_rule("water").is_none());
    }

    #[test]
    fn air_and_water_slow_mining_by_five() {
        let (r, t) = (block_rule("dirt").unwrap(), HAND);
        assert_eq!(break_ticks(&r, &t, false, true), 15);
        assert_eq!(break_ticks(&r, &t, false, false), 75);
        assert_eq!(break_ticks(&r, &t, true, false), 375);
    }

    #[test]
    fn harvest_requirements_gate_drops() {
        let none = |b: &str, t: &str| drops(b, &tool_of(t), 0.5).is_empty();
        assert!(none("stone", "hand"), "stone needs a pickaxe to drop anything");
        assert_eq!(drops("stone", &tool_of("wooden_pickaxe"), 0.5), vec![ItemStack::new("cobblestone", 1)]);
        assert!(none("iron_ore", "wooden_pickaxe") && !none("iron_ore", "stone_pickaxe"));
        assert!(none("diamond_ore", "stone_pickaxe") && !none("diamond_ore", "iron_pickaxe"));
        assert!(none("obsidian", "iron_pickaxe") && !none("obsidian", "diamond_pickaxe"));
        assert!(none("stone", "iron_axe"), "wrong tool kind does not satisfy the requirement");
        assert_eq!(drops("dirt", &HAND, 0.0), vec![ItemStack::new("dirt", 1)]);
        assert_eq!(drops("grass_block", &HAND, 0.0), vec![ItemStack::new("dirt", 1)]);
        assert_eq!(drops("oak_log", &HAND, 0.0), vec![ItemStack::new("oak_log", 1)]);
        assert_eq!(drops("coal_ore", &tool_of("wooden_pickaxe"), 0.0), vec![ItemStack::new("coal", 1)]);
    }

    #[test]
    fn chance_drops_use_the_roll() {
        assert_eq!(drops("oak_leaves", &HAND, 0.01), vec![ItemStack::new("oak_sapling", 1)]);
        assert_eq!(drops("oak_leaves", &HAND, 0.06), vec![ItemStack::new("stick", 1)]);
        assert!(drops("oak_leaves", &HAND, 0.9).is_empty());
        assert_eq!(drops("gravel", &HAND, 0.05), vec![ItemStack::new("flint", 1)]);
        assert_eq!(drops("gravel", &HAND, 0.5), vec![ItemStack::new("gravel", 1)]);
        let g = drops("glowstone", &HAND, 0.99); assert_eq!(g[0].count, 4);
        assert!((2..=4).contains(&drops("glowstone", &HAND, 0.0)[0].count));
    }

    #[test]
    fn tools_parse_and_block_items_are_distinguished() {
        let p = tool_of("minecraft:golden_pickaxe");
        assert_eq!((p.kind, p.level, p.speed), (ToolKind::Pickaxe, Some(0), 12.0));
        assert_eq!(tool_of("dirt"), HAND);
        assert_eq!(tool_of("stick"), HAND);
        assert!(is_block_item("cobblestone") && is_block_item("minecraft:oak_log"));
        assert!(!is_block_item("iron_pickaxe") && !is_block_item("stick") && !is_block_item("coal") && !is_block_item("water_bucket"));
    }
}
