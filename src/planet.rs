use crate::octree::{Bounds, NodeKey, CHUNK_EDGE};
use glam::{DQuat, DVec3};

#[derive(Clone, Debug)]
pub struct Planet {
    pub center: DVec3,
    pub radius: f64,
    pub surface_gravity: f64,
}
impl Planet {
    pub fn new(center: DVec3, radius: f64, surface_gravity: f64) -> Self {
        assert!(
            center.is_finite()
                && radius.is_finite()
                && radius > 0.0
                && surface_gravity.is_finite()
                && surface_gravity >= 0.0
        );
        Self {
            center,
            radius,
            surface_gravity,
        }
    }
    pub fn sdf(&self, position: DVec3) -> f64 {
        (position - self.center).length() - self.radius
    }
    pub fn up(&self, position: DVec3) -> DVec3 {
        (position - self.center).normalize_or_zero()
    }
    /// Outside: inverse-square. Inside: uniform-density sphere, finite at center.
    pub fn gravity(&self, position: DVec3) -> DVec3 {
        let delta = position - self.center;
        let r = delta.length();
        if r <= self.radius {
            -delta * (self.surface_gravity / self.radius)
        } else {
            -delta * (self.surface_gravity * self.radius.powi(2) / r.powi(3))
        }
    }
    pub fn root_bounds(&self) -> Bounds {
        let size = (self.radius * 2.1).log2().ceil().exp2();
        Bounds {
            min: self.center - DVec3::splat(size * 0.5),
            size,
        }
    }
    pub fn circular_speed(&self, altitude: f64) -> f64 {
        (self.surface_gravity * self.radius.powi(2) / (self.radius + altitude)).sqrt()
    }
}

/// Exact geodesic walking and parallel transport, independent of octree seams/poles.
#[derive(Clone, Debug)]
pub struct Walker {
    pub position: DVec3,
    pub forward: DVec3,
    pub eye_height: f64,
}
impl Walker {
    pub fn new(planet: &Planet) -> Self {
        Self {
            position: planet.center + DVec3::Y * (planet.radius + 1.7),
            forward: DVec3::Z,
            eye_height: 1.7,
        }
    }
    pub fn walk(&mut self, planet: &Planet, forward_meters: f64, strafe_meters: f64) {
        let up = planet.up(self.position);
        let forward = (self.forward - up * self.forward.dot(up)).normalize();
        let right = forward.cross(up).normalize();
        let tangent = forward * forward_meters + right * strafe_meters;
        if tangent.length_squared() < 1e-24 {
            return;
        }
        let q = DQuat::from_axis_angle(
            up.cross(tangent).normalize(),
            tangent.length() / planet.radius,
        );
        self.position = planet.center + (q * up) * (planet.radius + self.eye_height);
        self.forward = q * forward;
    }
    pub fn yaw(&mut self, planet: &Planet, radians: f64) {
        self.forward = DQuat::from_axis_angle(planet.up(self.position), radians) * self.forward;
    }
}

pub const SAMPLE_EDGE: usize = CHUNK_EDGE + 1;
/// 16^3 cells require 17^3 corner samples. Shared boundaries sample identical coordinates.
#[derive(Clone, Debug)]
pub struct TerrainChunk {
    pub key: NodeKey,
    pub bounds: Bounds,
    pub samples: Vec<f32>,
}
impl TerrainChunk {
    pub fn sample(&self, x: usize, y: usize, z: usize) -> f32 {
        self.samples[x + SAMPLE_EDGE * (y + SAMPLE_EDGE * z)]
    }
    fn new(key: NodeKey, bounds: Bounds, planet: &Planet) -> Self {
        let mut samples = Vec::with_capacity(SAMPLE_EDGE.pow(3));
        let step = bounds.size / CHUNK_EDGE as f64;
        for z in 0..SAMPLE_EDGE {
            for y in 0..SAMPLE_EDGE {
                for x in 0..SAMPLE_EDGE {
                    samples.push(
                        planet.sdf(bounds.min + DVec3::new(x as f64, y as f64, z as f64) * step)
                            as f32,
                    );
                }
            }
        }
        Self {
            key,
            bounds,
            samples,
        }
    }
}

#[derive(Clone, Debug)]
pub enum SurfaceNode {
    Empty,
    Solid,
    Branch(Box<[SurfaceNode; 8]>),
    Chunk(TerrainChunk),
}
#[derive(Clone, Debug)]
pub struct TerrainOctree {
    pub root: SurfaceNode,
    pub bounds: Bounds,
    pub depth: u8,
}
impl TerrainOctree {
    pub fn generate(planet: &Planet, depth: u8) -> Self {
        assert!(depth <= 5, "prototype residency limit: depth <= 5");
        let bounds = planet.root_bounds();
        Self {
            root: surface_node(planet, bounds, NodeKey::ROOT, depth),
            bounds,
            depth,
        }
    }
    pub fn chunks(&self) -> Vec<&TerrainChunk> {
        fn collect<'a>(n: &'a SurfaceNode, out: &mut Vec<&'a TerrainChunk>) {
            match n {
                SurfaceNode::Branch(c) => {
                    for n in c.iter() {
                        collect(n, out);
                    }
                }
                SurfaceNode::Chunk(c) => out.push(c),
                _ => {}
            }
        }
        let mut out = Vec::new();
        collect(&self.root, &mut out);
        out
    }
}
fn surface_node(p: &Planet, b: Bounds, key: NodeKey, depth: u8) -> SurfaceNode {
    // SDF is 1-Lipschitz: this sphere bound cannot discard an intersecting surface.
    let distance = p.sdf(b.center());
    let bound = b.size * 3.0_f64.sqrt() * 0.5;
    if distance > bound {
        return SurfaceNode::Empty;
    }
    if distance < -bound {
        return SurfaceNode::Solid;
    }
    if depth == 0 {
        return SurfaceNode::Chunk(TerrainChunk::new(key, b, p));
    }
    SurfaceNode::Branch(Box::new(std::array::from_fn(|i| {
        surface_node(p, b.child(i), key.child(i), depth - 1)
    })))
}

/// Conservative uniform-depth cut per planet prevents mixed-resolution cracks in v0.1.
/// Hysteresis prevents repeated rebuilds near thresholds. Local adaptive stitching is pending.
pub fn lod_depth(planet: &Planet, camera: DVec3, current: u8) -> u8 {
    let altitude = planet.sdf(camera).max(0.0) / planet.radius;
    match current {
        3 if altitude > 1.5 => 2,
        2 if altitude < 1.1 => 3,
        2 if altitude > 5.0 => 1,
        1 if altitude < 4.0 => 2,
        1..=3 => current,
        _ => 3,
    }
}
