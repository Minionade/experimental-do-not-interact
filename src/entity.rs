//! Entities with AABB-vs-voxel physics, stepped at 20 ticks per second.
use crate::access::BlockAccess;
use crate::rng::JavaRandom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Player, Pig, Item }

impl Kind {
    /// (width, height) of the collision box in blocks.
    pub fn size(self) -> (f64, f64) {
        match self { Kind::Player => (0.6, 1.8), Kind::Pig => (0.9, 0.9), Kind::Item => (0.25, 0.25) }
    }
}

#[derive(Clone, Debug)]
pub struct Entity {
    pub id: u32,
    pub kind: Kind,
    /// Feet position (centre of the bottom face).
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub on_ground: bool,
    pub in_water: bool,
    /// Desired horizontal movement direction (unit-ish), set by AI or player input.
    pub wish: [f64; 2],
    pub jump: bool,
    pub sneak: bool,
    pub flying: bool,
    /// Heading in radians; forward is (sin yaw, cos yaw). Updated from `wish` while moving.
    pub yaw: f64,
    wander_timer: u32,
}

const GRAVITY: f64 = 0.08;
const EPS: f64 = 1e-7;

fn solid(w: &impl BlockAccess, x: i32, y: i32, z: i32) -> bool {
    // Unloaded space counts as solid so entities don't wander off the loaded world.
    w.block(x, y, z).is_none_or(|b| b.is_solid())
}

impl Entity {
    fn half(&self) -> f64 { self.kind.size().0 / 2.0 }
    fn height(&self) -> f64 { self.kind.size().1 }

    /// Moves along one axis by `delta`, stopping at solid blocks. Returns the distance moved.
    fn move_axis(&mut self, w: &impl BlockAccess, axis: usize, delta: f64) -> f64 {
        if delta == 0.0 { return 0.0; }
        let (hw, h) = (self.half(), self.height());
        let lo = [self.pos[0] - hw, self.pos[1], self.pos[2] - hw];
        let hi = [self.pos[0] + hw, self.pos[1] + h, self.pos[2] + hw];
        let mut d = delta;
        let cell = |v: f64| v.floor() as i32;
        // Cell ranges overlapped by the swept box on the two other axes, and along the movement axis.
        let (mut r_lo, mut r_hi) = ([0i32; 3], [0i32; 3]);
        for a in 0..3 {
            let (a0, a1) = if a == axis { (lo[a].min(lo[a] + delta), hi[a].max(hi[a] + delta)) } else { (lo[a], hi[a]) };
            r_lo[a] = cell(a0 + EPS);
            r_hi[a] = cell(a1 - EPS);
        }
        for x in r_lo[0]..=r_hi[0] { for y in r_lo[1]..=r_hi[1] { for z in r_lo[2]..=r_hi[2] {
            if !solid(w, x, y, z) { continue; }
            let c = [x, y, z][axis] as f64;
            if delta > 0.0 { d = d.min((c - hi[axis]).max(0.0)); }
            else { d = d.max((c + 1.0 - lo[axis]).min(0.0)); }
        }}}
        self.pos[axis] += d;
        d
    }

    fn touching_water(&self, w: &impl BlockAccess) -> bool {
        let (hw, h) = (self.half(), self.height());
        let (x0, x1) = ((self.pos[0] - hw).floor() as i32, (self.pos[0] + hw).floor() as i32);
        let (z0, z1) = ((self.pos[2] - hw).floor() as i32, (self.pos[2] + hw).floor() as i32);
        let (y0, y1) = (self.pos[1].floor() as i32, (self.pos[1] + h - EPS).floor() as i32);
        (x0..=x1).any(|x| (y0..=y1).any(|y| (z0..=z1).any(|z|
            w.block(x, y, z).is_some_and(|b| b.name == "minecraft:water"))))
    }

    pub fn step(&mut self, w: &impl BlockAccess) {
        self.in_water = self.touching_water(w);
        if self.wish != [0.0; 2] { self.yaw = self.wish[0].atan2(self.wish[1]); }
        // Horizontal acceleration toward the wish direction.
        let accel = if self.flying { 0.1 } else if self.on_ground { 0.1 } else { 0.02 } * if self.in_water { 0.4 } else { 1.0 };
        self.vel[0] += self.wish[0] * accel;
        self.vel[2] += self.wish[1] * accel;
        if self.flying { self.vel[1] = if self.jump { 0.45 } else if self.sneak { -0.45 } else { 0.0 }; }
        else if self.jump {
            if self.in_water { self.vel[1] += 0.04; }
            else if self.on_ground { self.vel[1] = 0.42; }
        }
        let was_ground = self.on_ground;
        let dy = self.vel[1];
        let moved_y = self.move_axis(w, 1, dy);
        if moved_y != dy { self.on_ground = dy < 0.0; self.vel[1] = 0.0; } else { self.on_ground = false; }
        for axis in [0, 2] {
            let d = self.vel[axis];
            if self.move_axis(w, axis, d) != d { self.vel[axis] = 0.0; }
        }
        let _ = was_ground;
        // Friction and gravity (vanilla-style constants).
        let h_drag = if self.in_water { 0.8 } else if self.on_ground { 0.6 * 0.91 } else { 0.91 };
        self.vel[0] *= h_drag;
        self.vel[2] *= h_drag;
        if self.flying { self.on_ground = false; return; }
        self.vel[1] = if self.in_water { self.vel[1] * 0.8 - 0.02 } else { (self.vel[1] - GRAVITY) * 0.98 };
    }
}

pub struct EntityWorld {
    pub entities: Vec<Entity>,
    pub tick: u64,
    next_id: u32,
    rng: JavaRandom,
}

impl EntityWorld {
    pub fn new(seed: i64) -> Self { Self { entities: Vec::new(), tick: 0, next_id: 1, rng: JavaRandom::new(seed) } }

    pub fn spawn(&mut self, kind: Kind, pos: [f64; 3]) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.entities.push(Entity { id, kind, pos, vel: [0.0; 3], on_ground: false, in_water: false,
            wish: [0.0; 2], jump: false, sneak: false, flying: false, yaw: 0.0, wander_timer: 0 });
        id
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut Entity> { self.entities.iter_mut().find(|e| e.id == id) }

    pub fn tick(&mut self, w: &impl BlockAccess) {
        self.tick += 1;
        for e in &mut self.entities {
            // Freeze entities whose chunk isn't loaded rather than letting them fall through the void.
            if w.block(e.pos[0].floor() as i32, e.pos[1].floor() as i32, e.pos[2].floor() as i32).is_none() { continue; }
            if e.kind == Kind::Pig {
                // Simple wander AI: every 1-4 s pick a new heading, or stand still.
                if e.wander_timer == 0 {
                    e.wander_timer = 20 + self.rng.next_int_bound(60) as u32;
                    if self.rng.next_int_bound(3) == 0 { e.wish = [0.0; 2]; }
                    else {
                        let a = self.rng.next_double() * std::f64::consts::TAU;
                        e.wish = [a.cos() * 0.35, a.sin() * 0.35];
                    }
                }
                e.wander_timer -= 1;
                // Hop over one-block obstacles: jump when blocked horizontally.
                e.jump = e.on_ground && (e.wish[0] != 0.0 || e.wish[1] != 0.0) && {
                    let (dx, dz) = (e.wish[0].signum() * (e.half() + 0.1), e.wish[1].signum() * (e.half() + 0.1));
                    solid(w, (e.pos[0] + dx).floor() as i32, e.pos[1].floor() as i32, (e.pos[2] + dz).floor() as i32)
                };
            }
            e.step(w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::ChunkMap;
    use crate::chunk::{BlockState, Chunk};

    fn world() -> ChunkMap {
        let mut m = ChunkMap::new();
        for x in -2..2 { for z in -2..2 { m.insert((x, z), Chunk::empty(x, z)); } }
        let stone = BlockState::simple("minecraft:stone");
        for x in -32..32 { for z in -32..32 { m.set_block(x, 60, z, &stone); } } // top surface at y = 61
        m
    }

    #[test]
    fn falls_and_lands_on_the_ground() {
        let m = world();
        let mut ew = EntityWorld::new(1);
        let id = ew.spawn(Kind::Player, [0.5, 90.0, 0.5]);
        for _ in 0..200 { ew.tick(&m); }
        let e = ew.get_mut(id).unwrap();
        assert!((e.pos[1] - 61.0).abs() < 1e-6, "y = {}", e.pos[1]);
        assert!(e.on_ground);
    }

    #[test]
    fn walls_stop_horizontal_movement() {
        let mut m = world();
        let stone = BlockState::simple("minecraft:stone");
        for y in 61..64 { for z in -3..4 { m.set_block(5, y, z, &stone); } }
        let mut ew = EntityWorld::new(1);
        let id = ew.spawn(Kind::Player, [0.5, 61.0, 0.5]);
        ew.get_mut(id).unwrap().wish = [1.0, 0.0];
        for _ in 0..200 { ew.tick(&m); }
        let e = ew.get_mut(id).unwrap();
        assert!((e.pos[0] - (5.0 - 0.3)).abs() < 1e-6, "x = {}", e.pos[0]);
    }

    #[test]
    fn jumping_clears_one_block_but_not_two() {
        let mut m = world();
        let stone = BlockState::simple("minecraft:stone");
        m.set_block(3, 61, 0, &stone); // 1-block step
        m.set_block(8, 61, 0, &stone); m.set_block(8, 62, 0, &stone); // 2-block wall
        let mut ew = EntityWorld::new(1);
        let id = ew.spawn(Kind::Player, [0.5, 61.0, 0.5]);
        { let e = ew.get_mut(id).unwrap(); e.wish = [1.0, 0.0]; e.jump = true; }
        for _ in 0..200 { ew.tick(&m); }
        let e = ew.get_mut(id).unwrap();
        assert!((e.pos[0] - (8.0 - 0.3)).abs() < 1e-6, "hopped the step, stopped at the wall: x = {}", e.pos[0]);
    }

    #[test]
    fn water_slows_falling_and_allows_swimming_up() {
        let mut m = world();
        let water = BlockState::fluid("minecraft:water", 0);
        for y in 61..70 { for x in -3..4 { for z in -3..4 { m.set_block(x, y, z, &water); } } }
        let mut ew = EntityWorld::new(1);
        let id = ew.spawn(Kind::Player, [0.5, 68.0, 0.5]);
        for _ in 0..200 { ew.tick(&m); }
        assert!(ew.get_mut(id).unwrap().pos[1] < 62.0, "sinks to the bottom");
        ew.get_mut(id).unwrap().jump = true;
        for _ in 0..120 { ew.tick(&m); }
        assert!(ew.get_mut(id).unwrap().pos[1] > 66.0, "swims up");
    }

    #[test]
    fn pigs_wander_deterministically_and_stay_on_ground() {
        let run = || {
            let m = world();
            let mut ew = EntityWorld::new(9);
            let id = ew.spawn(Kind::Pig, [0.5, 61.0, 0.5]);
            for _ in 0..600 { ew.tick(&m); }
            ew.get_mut(id).unwrap().pos
        };
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        assert!((a[0] - 0.5).abs() + (a[2] - 0.5).abs() > 1.0, "pig moved");
        assert!((a[1] - 61.0).abs() < 1e-6);
    }
}
