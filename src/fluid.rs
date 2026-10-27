//! Tick-based fluid flow (simplified from vanilla behaviour).
//!
//! Level encoding: 0 = source, 1..=max flowing (distance from source), 8 = falling column.
//! Water spreads 7 blocks and updates every 5 ticks; lava spreads 3 and updates every 30.
//! Simplifications: no infinite water sources, no slope-seeking, lava/water contact rules are
//! "flowing lava + water => cobblestone, water onto lava source => obsidian".
use crate::access::BlockAccess;
use crate::chunk::BlockState;
use std::collections::BTreeMap;

const WATER: &str = "minecraft:water";
const LAVA: &str = "minecraft:lava";
const FALLING: u8 = 8;
const HORIZ: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

type Pos = (i32, i32, i32);

fn max_spread(name: &str) -> u8 { if name == LAVA { 3 } else { 7 } }
fn delay(name: &str) -> u64 { if name == LAVA { 30 } else { 5 } }
/// Distance from the nearest source: falling water counts as a source for sideways spread.
fn dist(level: u8) -> u8 { if level >= FALLING { 0 } else { level } }

#[derive(Default)]
pub struct FluidSim { due: BTreeMap<Pos, u64>, pub tick: u64 }

impl FluidSim {
    pub fn new() -> Self { Self::default() }

    /// Wake a position and its neighbours (call after any block change).
    pub fn notify(&mut self, p: Pos, w: &impl BlockAccess) {
        for q in [p, (p.0 + 1, p.1, p.2), (p.0 - 1, p.1, p.2), (p.0, p.1 + 1, p.2), (p.0, p.1 - 1, p.2), (p.0, p.1, p.2 + 1), (p.0, p.1, p.2 - 1)] {
            if let Some(b) = w.block(q.0, q.1, q.2) {
                if b.fluid_level().is_some() {
                    let d = delay(&b.name);
                    self.due.entry(q).or_insert(self.tick + d);
                }
            }
        }
    }

    pub fn is_idle(&self) -> bool { self.due.is_empty() }

    /// Places a source block and starts flowing.
    pub fn place_source(&mut self, w: &mut impl BlockAccess, p: Pos, name: &str) {
        w.set_block(p.0, p.1, p.2, &BlockState::fluid(name, 0));
        self.notify(p, w);
    }

    pub fn tick(&mut self, w: &mut impl BlockAccess) {
        self.tick += 1;
        let ready: Vec<Pos> = self.due.iter().filter(|(_, t)| **t <= self.tick).map(|(&p, _)| p).collect();
        for p in ready {
            self.due.remove(&p);
            self.update(w, p);
        }
    }

    fn replaceable(b: Option<&BlockState>) -> bool {
        matches!(b, Some(b) if !b.is_solid() && b.fluid_level().is_none())
    }

    /// True if fluid `name` placed above this block would flow down into it (open air, or a different fluid).
    fn open_below(b: Option<&BlockState>, name: &str) -> bool {
        Self::replaceable(b) || b.is_some_and(|b| b.fluid_level().is_some() && b.name != name)
    }

    fn update(&mut self, w: &mut impl BlockAccess, p: Pos) {
        let Some(cur) = w.block(p.0, p.1, p.2).cloned() else { return };
        let Some(level) = cur.fluid_level() else { return };
        let name = cur.name.clone();
        let max = max_spread(&name);

        // 1. Work out what this cell should be.
        let mut new_level = level;
        if level != 0 {
            let above_same = w.block(p.0, p.1 + 1, p.2).is_some_and(|b| b.name == name);
            let mut best: Option<u8> = None;
            if above_same { best = Some(FALLING); }
            else {
                for (dx, dz) in HORIZ {
                    let Some(n) = w.block(p.0 + dx, p.1, p.2 + dz) else { continue };
                    let Some(nl) = n.fluid_level().filter(|_| n.name == name) else { continue };
                    // Sources always feed sideways; others only if they rest on something (else they fall instead).
                    if nl != 0 && Self::open_below(w.block(p.0 + dx, p.1 - 1, p.2 + dz), &name) { continue; }
                    let v = dist(nl) + 1;
                    if v <= max && best.is_none_or(|b| b >= FALLING || v < b) { best = Some(v); }
                }
            }
            match best {
                None => {
                    w.set_block(p.0, p.1, p.2, &BlockState::air());
                    self.notify(p, w);
                    return;
                }
                Some(b) => new_level = b,
            }
            if new_level != level {
                w.set_block(p.0, p.1, p.2, &BlockState::fluid(&name, new_level));
                self.notify(p, w);
            }
        }

        // 2. Flow down if open; sources and supported cells also spread sideways.
        let below = w.block(p.0, p.1 - 1, p.2).cloned();
        let Some(below) = below else { return };
        let open = Self::open_below(Some(&below), &name);
        if open { self.flow_into(w, (p.0, p.1 - 1, p.2), &name, FALLING); }
        if open && new_level != 0 { return; }
        let d = dist(new_level);
        if d < max {
            for (dx, dz) in HORIZ {
                let q = (p.0 + dx, p.1, p.2 + dz);
                let nb = w.block(q.0, q.1, q.2);
                if Self::replaceable(nb) || nb.is_some_and(|b| b.fluid_level().is_some() && b.name != name) {
                    self.flow_into(w, q, &name, d + 1);
                } else if let Some(b) = nb {
                    // Existing same-fluid cell with a worse (higher) level gets improved.
                    if b.name == name && b.fluid_level().is_some_and(|l| l != 0 && l < FALLING && l > d + 1) {
                        self.flow_into(w, q, &name, d + 1);
                    }
                }
            }
        }
    }

    fn flow_into(&mut self, w: &mut impl BlockAccess, q: Pos, name: &str, level: u8) {
        let existing = w.block(q.0, q.1, q.2).cloned();
        let new = match existing.as_ref().and_then(|b| b.fluid_level().map(|l| (b.name.as_str(), l))) {
            Some((other, ol)) if other != name => {
                // Water meets lava: source lava -> obsidian, flowing lava -> cobblestone. Lava onto water -> stone.
                let result = if name == WATER && ol == 0 { "minecraft:obsidian" }
                    else if name == WATER { "minecraft:cobblestone" } else { "minecraft:stone" };
                BlockState::simple(result)
            }
            _ => BlockState::fluid(name, level),
        };
        if existing.as_ref() == Some(&new) { return; }
        w.set_block(q.0, q.1, q.2, &new);
        if new.fluid_level().is_some() { self.due.entry(q).or_insert(self.tick + delay(name)); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::ChunkMap;
    use crate::chunk::Chunk;

    fn flat_world() -> ChunkMap {
        let mut m = ChunkMap::new();
        for x in -2..2 { for z in -2..2 { m.insert((x, z), Chunk::empty(x, z)); } }
        let stone = BlockState::simple("minecraft:stone");
        for y in 60..70 { for x in -32..32 { for z in -32..32 { m.set_block(x, y, z, &stone); } } }
        m
    }
    fn count(m: &ChunkMap, name: &str) -> usize {
        let mut n = 0;
        for y in 60..90 { for x in -32..32 { for z in -32..32 {
            if m.block(x, y, z).is_some_and(|b| b.name == name) { n += 1; }
        }}}
        n
    }
    fn run(sim: &mut FluidSim, m: &mut ChunkMap, ticks: u32) { for _ in 0..ticks { sim.tick(m); } }

    #[test]
    fn water_spreads_seven_blocks_with_decreasing_level() {
        let mut m = flat_world();
        let mut sim = FluidSim::new();
        sim.place_source(&mut m, (0, 70, 0), WATER);
        run(&mut sim, &mut m, 200);
        for d in 0..=7 { assert_eq!(m.block(d, 70, 0).unwrap().fluid_level(), Some(d as u8), "dist {d}"); }
        assert!(m.block(8, 70, 0).unwrap().is_air());
        assert!(m.block(0, 70, 8).unwrap().is_air());
        // Diamond (manhattan) region of radius 7: 1 + 4*(1+..+7) = 113 cells.
        assert_eq!(count(&m, WATER), 113);
        assert!(sim.is_idle());
    }

    #[test]
    fn removing_source_drains_everything() {
        let mut m = flat_world();
        let mut sim = FluidSim::new();
        sim.place_source(&mut m, (0, 70, 0), WATER);
        run(&mut sim, &mut m, 200);
        m.set_block(0, 70, 0, &BlockState::air());
        sim.notify((0, 70, 0), &m);
        run(&mut sim, &mut m, 600);
        assert_eq!(count(&m, WATER), 0);
        assert!(sim.is_idle());
    }

    #[test]
    fn water_falls_off_ledges_and_spreads_on_the_floor() {
        let mut m = flat_world();
        // Dig a 9-deep pit at x=2 (stone floor remains at y=60).
        for y in 61..70 { m.set_block(2, y, 0, &BlockState::air()); }
        let mut sim = FluidSim::new();
        sim.place_source(&mut m, (0, 70, 0), WATER);
        run(&mut sim, &mut m, 300);
        for y in 61..70 { assert!(m.block(2, y, 0).unwrap().fluid_level().is_some(), "pit y={y}"); }
        assert_eq!(m.block(2, 69, 0).unwrap().fluid_level(), Some(FALLING));
    }

    #[test]
    fn lava_is_shorter_and_meets_water_as_cobblestone() {
        let mut m = flat_world();
        let mut sim = FluidSim::new();
        sim.place_source(&mut m, (0, 70, 0), LAVA);
        run(&mut sim, &mut m, 400);
        assert_eq!(m.block(3, 70, 0).unwrap().fluid_level(), Some(3));
        assert!(m.block(4, 70, 0).unwrap().is_air());
        sim.place_source(&mut m, (6, 70, 0), WATER);
        run(&mut sim, &mut m, 400);
        let n = (1..=5).filter(|&x| m.block(x, 70, 0).unwrap().name == "minecraft:cobblestone").count();
        assert!(n >= 1, "expected cobblestone where fluids meet");
    }
}
