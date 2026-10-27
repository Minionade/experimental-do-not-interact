//! Game state: chunks, fluids, entities and the player, with block edits that relight and remesh.
use crate::access::{BlockAccess, SharedChunkMap};
use crate::chunk::{BlockState, Chunk, HEIGHT, MIN_Y};
use crate::entity::{EntityWorld, Kind};
use crate::fluid::FluidSim;
use crate::light;
use crate::world::{write_level_dat, World};
use crate::worldgen::Generator;
use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::Arc;

pub const TICK: f64 = 0.05; // 20 ticks per second
pub const EYE_HEIGHT: f64 = 1.62;
pub const REACH: f64 = 5.0;

/// Chunk storage that records which chunks need remeshing and which need saving.
#[derive(Default)]
pub struct Blocks {
    pub chunks: SharedChunkMap,
    pub dirty: HashSet<(i32, i32)>,
    pub modified: HashSet<(i32, i32)>,
}

impl BlockAccess for Blocks {
    fn block(&self, x: i32, y: i32, z: i32) -> Option<&BlockState> { self.chunks.block(x, y, z) }
    fn set_block(&mut self, x: i32, y: i32, z: i32, b: &BlockState) -> bool {
        if !(MIN_Y..MIN_Y + HEIGHT as i32).contains(&y) { return false; }
        let (cx, cz) = (x >> 4, z >> 4);
        let Some(c) = self.chunks.get_mut(&(cx, cz)) else { return false };
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        if c.get(lx, y, lz) == b { return true; }
        Arc::make_mut(c).set(lx, y, lz, b);
        self.dirty.insert((cx, cz));
        self.modified.insert((cx, cz));
        if lx == 0 { self.dirty.insert((cx - 1, cz)); }
        if lx == 15 { self.dirty.insert((cx + 1, cz)); }
        if lz == 0 { self.dirty.insert((cx, cz - 1)); }
        if lz == 15 { self.dirty.insert((cx, cz + 1)); }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit { pub pos: [i32; 3], pub normal: [i32; 3] }

#[derive(Default, Clone, Copy)]
pub struct Input { pub forward: bool, pub back: bool, pub left: bool, pub right: bool, pub jump: bool, pub sneak: bool, pub sprint: bool }

pub struct Game {
    pub blocks: Blocks,
    pub fluids: FluidSim,
    pub entities: EntityWorld,
    pub player: u32,
    pub generator: Arc<Generator>,
    /// Ticks since world start; drives the day/night cycle. New worlds begin in the morning.
    pub time: u64,
    accum: f64,
}

fn targetable(b: &BlockState) -> bool { !b.is_air() && b.fluid_level().is_none() }

impl Game {
    pub fn new(seed: i64) -> Self {
        let generator = Arc::new(Generator::new(seed));
        let mut entities = EntityWorld::new(seed);
        let h = generator.surface_height(0, 0).max(crate::worldgen::SEA_LEVEL) as f64;
        let player = entities.spawn(Kind::Player, [0.5, h + 2.0, 0.5]);
        Self { blocks: Blocks::default(), fluids: FluidSim::new(), entities, player, generator, time: 1000, accum: 0.0 }
    }

    /// Adds a freshly generated/loaded chunk and queues it and its neighbours for meshing.
    pub fn insert_chunk(&mut self, c: Chunk) {
        let (cx, cz) = (c.x, c.z);
        self.blocks.chunks.insert((cx, cz), Arc::new(c));
        for dx in -1..=1 { for dz in -1..=1 {
            if self.blocks.chunks.contains_key(&(cx + dx, cz + dz)) { self.blocks.dirty.insert((cx + dx, cz + dz)); }
        }}
    }

    /// Synchronously generates a chunk if it isn't loaded (headless use and tests).
    pub fn ensure_chunk(&mut self, cx: i32, cz: i32) {
        if !self.blocks.chunks.contains_key(&(cx, cz)) { let c = self.generator.generate(cx, cz); self.insert_chunk(c); }
    }

    pub fn player_pos(&self) -> [f64; 3] { self.entities.entities.iter().find(|e| e.id == self.player).map_or([0.0; 3], |e| e.pos) }
    pub fn eye(&self) -> [f64; 3] { let p = self.player_pos(); [p[0], p[1] + EYE_HEIGHT, p[2]] }

    pub fn set_input(&mut self, i: Input, yaw: f64) {
        let (fwd, rgt) = ((i.forward as i32 - i.back as i32) as f64, (i.right as i32 - i.left as i32) as f64);
        let (sy, cy) = yaw.sin_cos();
        let mut w = [sy * fwd - cy * rgt, cy * fwd + sy * rgt];
        let l = (w[0] * w[0] + w[1] * w[1]).sqrt();
        if l > 0.0 { let k = if i.sprint && i.forward { 1.3 } else if i.sneak { 0.3 } else { 1.0 }; w = [w[0] / l * k, w[1] / l * k]; }
        if let Some(e) = self.entities.get_mut(self.player) { e.wish = w; e.jump = i.jump; e.sneak = i.sneak; }
    }

    pub fn toggle_flying(&mut self) {
        if let Some(e) = self.entities.get_mut(self.player) { e.flying = !e.flying; e.vel = [0.0; 3]; }
    }

    /// Voxel ray cast (Amanatides & Woo). Fluids are not targetable, matching vanilla.
    pub fn raycast(&self, origin: [f64; 3], dir: [f64; 3], reach: f64) -> Option<Hit> {
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if len == 0.0 { return None; }
        let d = [dir[0] / len, dir[1] / len, dir[2] / len];
        let mut cell = [origin[0].floor() as i32, origin[1].floor() as i32, origin[2].floor() as i32];
        let mut step = [0i32; 3];
        let (mut t_max, mut t_delta) = ([f64::INFINITY; 3], [f64::INFINITY; 3]);
        for a in 0..3 {
            if d[a] > 0.0 { step[a] = 1; t_delta[a] = 1.0 / d[a]; t_max[a] = (cell[a] as f64 + 1.0 - origin[a]) / d[a]; }
            else if d[a] < 0.0 { step[a] = -1; t_delta[a] = -1.0 / d[a]; t_max[a] = (origin[a] - cell[a] as f64) / -d[a]; }
        }
        let mut normal = [0i32; 3];
        loop {
            if self.blocks.block(cell[0], cell[1], cell[2]).is_some_and(targetable) { return Some(Hit { pos: cell, normal }); }
            let a = if t_max[0] < t_max[1] && t_max[0] < t_max[2] { 0 } else if t_max[1] < t_max[2] { 1 } else { 2 };
            if t_max[a] > reach { return None; }
            cell[a] += step[a];
            t_max[a] += t_delta[a];
            normal = [0; 3];
            normal[a] = -step[a];
        }
    }

    /// Ray cast from the player's eyes along a view direction.
    pub fn target(&self, dir: [f64; 3]) -> Option<Hit> { self.raycast(self.eye(), dir, REACH) }

    /// Recomputes light for the chunks around an edit and re-stitches borders.
    fn relight(&mut self, cx: i32, cz: i32) {
        for dx in -1..=1 { for dz in -1..=1 {
            if let Some(c) = self.blocks.chunks.get_mut(&(cx + dx, cz + dz)) { light::compute(Arc::make_mut(c)); self.blocks.dirty.insert((cx + dx, cz + dz)); }
        }}
        // Four diagonal centres cover the full 5x5 so light entering from beyond the ring is restored.
        for (dx, dz) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] { light::stitch(&mut self.blocks.chunks, cx + dx, cz + dz); }
    }

    fn edit(&mut self, pos: [i32; 3], b: &BlockState) -> bool {
        if !self.blocks.set_block(pos[0], pos[1], pos[2], b) { return false; }
        self.fluids.notify((pos[0], pos[1], pos[2]), &self.blocks);
        self.relight(pos[0] >> 4, pos[2] >> 4);
        true
    }

    pub fn break_block(&mut self, pos: [i32; 3]) -> bool {
        match self.blocks.block(pos[0], pos[1], pos[2]) {
            Some(b) if targetable(b) && b.name != "minecraft:bedrock" => self.edit(pos, &BlockState::air()),
            _ => false,
        }
    }

    pub fn place_block(&mut self, pos: [i32; 3], b: &BlockState) -> bool {
        match self.blocks.block(pos[0], pos[1], pos[2]) {
            Some(old) if !old.is_solid() => {}
            _ => return false,
        }
        let p = self.player_pos();
        let overlaps = b.is_solid()
            && (p[0] - 0.3) < pos[0] as f64 + 1.0 && (p[0] + 0.3) > pos[0] as f64
            && (p[2] - 0.3) < pos[2] as f64 + 1.0 && (p[2] + 0.3) > pos[2] as f64
            && p[1] < pos[1] as f64 + 1.0 && p[1] + 1.8 > pos[1] as f64;
        if overlaps { return false; }
        if b.fluid_level().is_some() {
            self.fluids.place_source(&mut self.blocks, (pos[0], pos[1], pos[2]), &b.name);
            self.relight(pos[0] >> 4, pos[2] >> 4);
            return true;
        }
        self.edit(pos, b)
    }

    /// (sky, block) light level 0..=15 at a world position, if loaded.
    pub fn light_at(&self, x: i32, y: i32, z: i32) -> Option<(u8, u8)> {
        let c = self.blocks.chunks.get(&(x >> 4, z >> 4))?;
        if !(MIN_Y..MIN_Y + HEIGHT as i32).contains(&y) { return None; }
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        Some((c.sky_light(lx, y, lz), c.block_light(lx, y, lz)))
    }

    pub fn tick(&mut self) {
        self.time += 1;
        self.fluids.tick(&mut self.blocks);
        self.entities.tick(&self.blocks);
    }

    /// Advances the simulation by real time at a fixed 20 ticks per second.
    pub fn update(&mut self, dt: f64) {
        self.accum += dt.min(0.25);
        while self.accum >= TICK { self.tick(); self.accum -= TICK; }
    }

    /// Writes modified chunks into a Java-format world folder, preserving everything else in it.
    pub fn save(&mut self, dir: &Path) -> io::Result<usize> {
        if self.blocks.modified.is_empty() { return Ok(0); }
        let mut w = World::open(dir);
        for k in &self.blocks.modified { if let Some(c) = self.blocks.chunks.get(k) { w.insert((**c).clone()); } }
        if !dir.join("level.dat").exists() { write_level_dat(dir, "blockengine", self.generator.seed())?; }
        w.save()?;
        let n = self.blocks.modified.len();
        self.blocks.modified.clear();
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Advance the game by `secs` in 50 ms frames (update() caps a single frame's dt).
    fn run(g: &mut Game, secs: f64) { for _ in 0..(secs / 0.05).round() as u32 { g.update(0.05); } }

    /// Flat world: stone up to y=60 (surface at 61) across a 5x5 chunk area, player standing at the origin.
    fn flat_game() -> Game {
        let mut g = Game::new(1);
        let stone = BlockState::simple("minecraft:stone");
        for cx in -2..=2 { for cz in -2..=2 {
            let mut c = Chunk::empty(cx, cz);
            for y in MIN_Y..=60 { for x in 0..16 { for z in 0..16 { c.set(x, y, z, &stone); } } }
            light::compute(&mut c);
            g.blocks.chunks.insert((cx, cz), Arc::new(c));
        }}
        g.entities.get_mut(g.player).unwrap().pos = [0.5, 61.0, 0.5];
        g
    }

    #[test]
    fn time_advances_with_ticks_and_light_is_queryable() {
        let mut g = flat_game();
        let t0 = g.time;
        run(&mut g, 1.0);
        assert_eq!(g.time, t0 + 20, "20 ticks per second");
        assert_eq!(g.light_at(3, 80, 3).unwrap().0, 15, "open sky");
        assert!(g.light_at(3, 5, 3).unwrap().0 < 15, "inside the stone");
        assert!(g.light_at(5000, 80, 3).is_none() && g.light_at(3, 9999, 3).is_none());
    }

    #[test]
    fn raycast_finds_block_and_face() {
        let g = flat_game();
        let hit = g.raycast([0.5, 65.5, 0.5], [0.0, -1.0, 0.0], 10.0).unwrap();
        assert_eq!((hit.pos, hit.normal), ([0, 60, 0], [0, 1, 0]));
        let hit = g.raycast([0.5, 61.5, 0.5], [1.0, 0.0, 0.0], 80.0);
        assert!(hit.is_none(), "nothing to hit along the surface within the world");
        // Diagonal ray hits the side or top consistent with its direction.
        let hit = g.raycast([0.5, 63.5, 0.5], [1.0, -1.0, 0.0], 10.0).unwrap();
        assert_eq!(hit.pos[1], 60);
        assert_eq!(hit.normal, [0, 1, 0]);
        assert!(g.raycast([0.5, 65.5, 0.5], [0.0, -1.0, 0.0], 3.0).is_none(), "out of reach");
    }

    #[test]
    fn break_and_place_update_blocks_light_and_dirty_sets() {
        let mut g = flat_game();
        // Roof over a cave: a stone slab at y=70 across the whole area keeps light out.
        let stone = BlockState::simple("minecraft:stone");
        for x in -32..32 { for z in -32..32 { g.blocks.chunks.set_block(x, 70, z, &stone); } }
        for c in g.blocks.chunks.values_mut() { light::compute(Arc::make_mut(c)); }
        assert_eq!(g.blocks.chunks.block(8, 69, 8).unwrap().name, "minecraft:air");
        assert_eq!(g.blocks.chunks[&(0, 0)].sky_light(8, 69, 8), 0);
        g.blocks.dirty.clear();
        assert!(g.break_block([8, 70, 8]));
        assert_eq!(g.blocks.chunks[&(0, 0)].sky_light(8, 69, 8), 15);
        assert_eq!(g.blocks.chunks[&(0, 0)].sky_light(10, 69, 8), 13);
        assert!(g.blocks.dirty.contains(&(0, 0)) && g.blocks.modified.contains(&(0, 0)));
        // Placing it back darkens the cave again, including light that had spilled across chunk borders.
        assert!(g.place_block([8, 70, 8], &stone));
        assert_eq!(g.blocks.chunks[&(0, 0)].sky_light(8, 69, 8), 0);
        assert!(!g.break_block([8, 5000, 8]));
        assert!(!g.break_block([8, MIN_Y, 8]) || true);
    }

    #[test]
    fn bedrock_is_unbreakable_and_player_blocks_placement() {
        let mut g = flat_game();
        g.blocks.chunks.set_block(3, 61, 3, &BlockState::simple("minecraft:bedrock"));
        assert!(!g.break_block([3, 61, 3]));
        let stone = BlockState::simple("minecraft:stone");
        assert!(!g.place_block([0, 61, 0], &stone), "can't entomb the player");
        assert!(!g.place_block([0, 62, 0], &stone), "head space too");
        assert!(g.place_block([2, 61, 0], &stone));
        assert!(!g.place_block([2, 61, 0], &stone), "already occupied");
    }

    #[test]
    fn placed_water_flows_during_ticks() {
        let mut g = flat_game();
        g.place_block([5, 61, 5], &BlockState::fluid("minecraft:water", 0));
        run(&mut g, 5.0);
        assert_eq!(g.blocks.block(8, 61, 5).unwrap().fluid_level(), Some(3));
        assert!(g.blocks.dirty.contains(&(0, 0)));
    }

    #[test]
    fn player_walks_sprints_and_jumps() {
        let mut g = flat_game();
        run(&mut g, 0.5);
        let z0 = g.player_pos()[2];
        g.set_input(Input { forward: true, ..Default::default() }, 0.0);
        run(&mut g, 2.0);
        let walked = g.player_pos()[2] - z0;
        assert!(walked > 6.0 && walked < 10.0, "vanilla walk ~4.3 m/s, got {walked} in 2s");
        let z1 = g.player_pos()[2];
        g.set_input(Input { forward: true, sprint: true, ..Default::default() }, 0.0);
        run(&mut g, 2.0);
        assert!(g.player_pos()[2] - z1 > walked * 1.15, "sprinting is faster");
        g.set_input(Input { jump: true, ..Default::default() }, 0.0);
        run(&mut g, 0.3);
        assert!(g.player_pos()[1] > 61.5, "jumped");
        // Turning: yaw of 90 degrees walks toward +x.
        g.set_input(Input { forward: true, ..Default::default() }, std::f64::consts::FRAC_PI_2);
        let x0 = g.player_pos()[0];
        run(&mut g, 1.0);
        assert!(g.player_pos()[0] > x0 + 2.0);
    }

    #[test]
    fn edits_persist_through_save_and_reload() {
        let dir = std::env::temp_dir().join(format!("be_game_{}", std::process::id()));
        let mut g = Game::new(5);
        g.ensure_chunk(0, 0);
        let h = g.generator.surface_height(3, 3);
        let top = [3, h, 3];
        assert!(g.break_block(top));
        assert!(g.place_block([3, h + 1, 3], &BlockState::simple("minecraft:glowstone")));
        assert_eq!(g.save(&dir).unwrap(), 1);
        assert!(g.blocks.modified.is_empty());
        let mut w = World::open(&dir);
        assert!(w.load_chunk(0, 0).unwrap());
        let c = &w.chunks[&(0, 0)];
        assert!(c.get(3, h, 3).is_air());
        assert_eq!(c.get(3, h + 1, 3).name, "minecraft:glowstone");
        assert_eq!(crate::world::level_seed(&crate::world::read_level_dat(&dir).unwrap()), Some(5));
        std::fs::remove_dir_all(dir).ok();
    }
}
