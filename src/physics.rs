//! Rapier f64 rigid bodies in one inertial frame; per-body radial gravity.
use crate::mods::{Command, Event, ModHost};
use crate::{
    octree::{BlockOctree, AIR},
    planet::Planet,
};
use glam::{DQuat, DVec3, IVec3};
use rapier3d_f64::prelude::*;
use std::collections::HashMap;

pub const FIXED_DT: f64 = 1.0 / 120.0;
pub type BodyHandle = RigidBodyHandle;
fn rv(v: DVec3) -> Vector<Real> {
    vector![v.x, v.y, v.z]
}
fn gv(v: &Vector<Real>) -> DVec3 {
    DVec3::new(v.x, v.y, v.z)
}

pub struct Physics {
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad: BroadPhaseMultiSap,
    narrow: NarrowPhase,
    pub bodies: RigidBodySet,
    colliders: ColliderSet,
    joints: ImpulseJointSet,
    multibody: MultibodyJointSet,
    ccd: CCDSolver,
    pub planets: Vec<Planet>,
    accumulator: f64,
    pub dropped_seconds: f64,
    pub mods: ModHost,
    pub tick: u64,
    pending_commands: Vec<Command>,
}
impl Physics {
    pub fn new(planets: Vec<Planet>) -> Self {
        let mut s = Self {
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad: BroadPhaseMultiSap::new(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            joints: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            planets,
            accumulator: 0.0,
            dropped_seconds: 0.0,
            mods: ModHost::default(),
            tick: 0,
            pending_commands: Vec::new(),
        };
        for p in &s.planets {
            s.colliders.insert(
                ColliderBuilder::ball(p.radius)
                    .translation(rv(p.center))
                    .friction(0.8)
                    .build(),
            );
        }
        s
    }
    pub fn spawn_ball(&mut self, position: DVec3, velocity: DVec3, radius: f64) -> BodyHandle {
        let handle = self.bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(rv(position))
                .linvel(rv(velocity))
                .can_sleep(false)
                .ccd_enabled(true)
                .build(),
        );
        self.colliders.insert_with_parent(
            ColliderBuilder::ball(radius).density(1.0).build(),
            handle,
            &mut self.bodies,
        );
        handle
    }
    /// Compound collision uses exactly the occupied octree blocks; no bounding-box collider.
    pub fn spawn_structure(
        &mut self,
        tree: &BlockOctree,
        min: IVec3,
        max: IVec3,
        position: DVec3,
    ) -> Result<BodyHandle, String> {
        if !tree.contains(min) || !tree.contains(max - IVec3::ONE) || !max.cmpgt(min).all() {
            return Err("invalid structure extraction bounds".into());
        }
        let volume = (max - min).as_i64vec3();
        if volume.x * volume.y * volume.z > 32768 {
            return Err("prototype structure extraction is limited to 32768 cells".into());
        }
        let mut parts = Vec::new();
        for z in min.z..max.z {
            for y in min.y..max.y {
                for x in min.x..max.x {
                    if tree.get(IVec3::new(x, y, z)).unwrap_or(AIR) != AIR {
                        parts.push((
                            Isometry::translation(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5),
                            SharedShape::cuboid(0.5, 0.5, 0.5),
                        ));
                    }
                }
            }
        }
        if parts.is_empty() {
            return Err("cannot assemble an empty structure".into());
        }
        let h = self.bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(rv(position))
                .can_sleep(false)
                .ccd_enabled(true)
                .build(),
        );
        self.colliders.insert_with_parent(
            ColliderBuilder::compound(parts)
                .density(30.0)
                .friction(0.8)
                .restitution(0.05)
                .build(),
            h,
            &mut self.bodies,
        );
        self.pending_commands
            .extend(self.mods.dispatch(&Event::StructureSpawned(h)));
        Ok(h)
    }
    pub fn pose(&self, h: BodyHandle) -> (DVec3, DQuat) {
        let b = &self.bodies[h];
        let q = b.rotation().quaternion();
        (gv(b.translation()), DQuat::from_xyzw(q.i, q.j, q.k, q.w))
    }
    pub fn velocity(&self, h: BodyHandle) -> DVec3 {
        gv(self.bodies[h].linvel())
    }
    /// One fixed step. `thrust` is a world-space acceleration command.
    pub fn step(&mut self, thrust: Option<(BodyHandle, DVec3)>) {
        let mut commands = std::mem::take(&mut self.pending_commands);
        commands.extend(self.mods.dispatch(&Event::FixedTick {
            tick: self.tick,
            dt: FIXED_DT,
        }));
        let mut accelerations = HashMap::new();
        if let Some((body, acceleration)) = thrust {
            commands.push(Command::Accelerate { body, acceleration });
        }
        for command in commands {
            match command {
                Command::Accelerate { body, acceleration }
                    if acceleration.is_finite() && self.bodies.contains(body) =>
                {
                    *accelerations.entry(body).or_insert(DVec3::ZERO) += acceleration;
                }
                _ => {}
            }
        }
        // Velocity Verlet: gravity half-kick, Rapier drift/contact solve, half-kick.
        // Applying one stale gravity force through Rapier's internal substeps causes
        // secular orbital energy drift. Splitting the field keeps free orbits bounded.
        self.gravity_kick(&accelerations);
        let parameters = IntegrationParameters {
            dt: FIXED_DT,
            ..Default::default()
        };
        self.pipeline.step(
            &Vector::zeros(),
            &parameters,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.joints,
            &mut self.multibody,
            &mut self.ccd,
            None,
            &(),
            &(),
        );
        self.gravity_kick(&accelerations);
        self.tick += 1;
    }
    fn gravity_kick(&mut self, accelerations: &HashMap<BodyHandle, DVec3>) {
        for (h, b) in self.bodies.iter_mut() {
            if !b.is_dynamic() {
                continue;
            }
            let position = gv(&b.center_of_mass().coords);
            let mut acceleration = self
                .planets
                .iter()
                .fold(DVec3::ZERO, |a, p| a + p.gravity(position));
            if let Some(command) = accelerations.get(&h) {
                acceleration += *command;
            }
            b.reset_forces(false);
            b.set_linvel(*b.linvel() + rv(acceleration * (FIXED_DT * 0.5)), true);
        }
    }
    pub fn advance(&mut self, real_dt: f64, thrust: Option<(BodyHandle, DVec3)>) -> usize {
        if !real_dt.is_finite() || real_dt < 0.0 {
            return 0;
        }
        let accepted = real_dt.min(0.1);
        self.dropped_seconds += real_dt - accepted;
        self.accumulator += accepted;
        let mut steps = 0;
        while self.accumulator + 1e-12 >= FIXED_DT && steps < 12 {
            self.step(thrust);
            self.accumulator -= FIXED_DT;
            steps += 1;
        }
        self.accumulator = self.accumulator.max(0.0);
        steps
    }
}

pub fn example_ship() -> BlockOctree {
    let mut tree = BlockOctree::new(IVec3::splat(-16), 32).unwrap();
    for z in -3..=3 {
        for x in -2..=2 {
            tree.set(IVec3::new(x, 0, z), 1).unwrap();
        }
    }
    for z in -1..=1 {
        tree.set(IVec3::new(0, 1, z), 2).unwrap();
    }
    tree
}
