//! Editable cubic world with registered state IDs and boundary-aware chunk invalidation.
use crate::{
    octree::{BlockOctree, AIR},
    state_registry::StateRegistry,
};
use glam::{DVec3, IVec3};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};
pub type ChunkKey = [i32; 3];
pub struct BlockWorld {
    pub registry: Arc<StateRegistry>,
    tree: BlockOctree,
    dirty: BTreeSet<ChunkKey>,
}
#[derive(Serialize, Deserialize)]
struct Save {
    format: u32,
    minecraft: String,
    registry_fingerprint: String,
    tree: serde_json::Value,
}
#[derive(Debug, PartialEq)]
pub struct Hit {
    pub cell: IVec3,
    pub normal: IVec3,
    pub distance: f64,
    pub state: u32,
}
impl BlockWorld {
    pub fn new(registry: Arc<StateRegistry>, min: IVec3, size: i32) -> Result<Self, String> {
        if min.rem_euclid(IVec3::splat(16)) != IVec3::ZERO {
            return Err("world origin must align to cubic chunks".into());
        }
        Ok(Self {
            registry,
            tree: BlockOctree::new(min, size)?,
            dirty: BTreeSet::new(),
        })
    }
    pub fn get(&self, p: IVec3) -> Option<u32> {
        self.tree.get(p)
    }
    pub fn contains(&self, p: IVec3) -> bool {
        self.tree.contains(p)
    }
    pub fn node_count(&self) -> usize {
        self.tree.node_count()
    }
    pub fn set(&mut self, p: IVec3, state: u32) -> Result<(), String> {
        if self.registry.get(state).is_none() {
            return Err(format!("unregistered state {state}"));
        }
        if self.get(p) == Some(state) {
            return Ok(());
        }
        self.tree.set(p, state)?;
        let chunk = p.div_euclid(IVec3::splat(16));
        let local = p.rem_euclid(IVec3::splat(16));
        self.dirty.insert(chunk.to_array());
        for axis in 0..3 {
            for (edge, step) in [(0, -1), (15, 1)] {
                if local[axis] == edge {
                    let mut n = chunk;
                    n[axis] += step;
                    self.dirty.insert(n.to_array());
                }
            }
        }
        Ok(())
    }
    pub fn take_dirty(&mut self) -> BTreeSet<ChunkKey> {
        std::mem::take(&mut self.dirty)
    }
    pub fn active_chunks(&self, limit: usize) -> Result<BTreeSet<ChunkKey>, String> {
        let mut result = BTreeSet::new();
        let mut overflow = false;
        self.tree.visit_nonempty_regions(|min, size| {
            if overflow {
                return;
            }
            let edge = (size / 16) as usize;
            let count = edge.checked_pow(3).unwrap_or(usize::MAX);
            if count > limit.saturating_sub(result.len()) {
                overflow = true;
                return;
            }
            for z in (0..size).step_by(16) {
                for y in (0..size).step_by(16) {
                    for x in (0..size).step_by(16) {
                        result.insert(
                            (min + IVec3::new(x, y, z))
                                .div_euclid(IVec3::splat(16))
                                .to_array(),
                        );
                    }
                }
            }
        });
        if overflow {
            Err("active chunks exceed budget; use streaming region selection".into())
        } else {
            Ok(result)
        }
    }
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(&Save {
            format: 1,
            minecraft: "26.3".into(),
            registry_fingerprint: self.registry.fingerprint().into(),
            tree: serde_json::from_str(&self.tree.to_json()?).map_err(|e| e.to_string())?,
        })
        .map_err(|e| e.to_string())
    }
    pub fn from_json(registry: Arc<StateRegistry>, text: &str) -> Result<Self, String> {
        let save: Save = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if save.format != 1
            || save.minecraft != "26.3"
            || save.registry_fingerprint != registry.fingerprint()
        {
            return Err("save version or registry fingerprint mismatch".into());
        }
        let tree = BlockOctree::from_json(&save.tree.to_string())?;
        if tree.origin().rem_euclid(IVec3::splat(16)) != IVec3::ZERO {
            return Err("unaligned world origin".into());
        }
        let mut invalid = false;
        tree.visit_ids(|id| invalid |= registry.get(id).is_none());
        if invalid {
            return Err("save contains unregistered state IDs".into());
        }
        Ok(Self {
            registry,
            tree,
            dirty: BTreeSet::new(),
        })
    }
    /// Voxel DDA: candidate cell selection; exact non-cubic selection shapes are separate.
    pub fn raycast(&self, origin: DVec3, direction: DVec3, reach: f64) -> Option<Hit> {
        if !origin.is_finite()
            || !direction.is_finite()
            || !reach.is_finite()
            || !(0. ..=1024.).contains(&reach)
            || direction.length_squared() < 1e-24
        {
            return None;
        }
        let direction = direction.normalize();
        let floor = origin.floor();
        if floor.min_element() < (i32::MIN + 2048) as f64
            || floor.max_element() > (i32::MAX - 2048) as f64
        {
            return None;
        }
        let mut cell = floor.as_ivec3();
        let mut delta = DVec3::splat(f64::INFINITY);
        let mut next = delta;
        let mut step = IVec3::ZERO;
        for a in 0..3 {
            if direction[a] != 0. {
                step[a] = if direction[a] > 0. { 1 } else { -1 };
                delta[a] = 1. / direction[a].abs();
                next[a] =
                    ((cell[a] + if step[a] > 0 { 1 } else { 0 }) as f64 - origin[a]) / direction[a];
            }
        }
        let mut distance = 0.;
        let mut normal = IVec3::ZERO;
        loop {
            let state = self.get(cell).unwrap_or(AIR);
            if !self.registry.is_air(state) {
                return Some(Hit {
                    cell,
                    normal,
                    distance,
                    state,
                });
            }
            let axis = if next.x <= next.y && next.x <= next.z {
                0
            } else if next.y <= next.z {
                1
            } else {
                2
            };
            distance = next[axis];
            if distance > reach {
                return None;
            }
            normal = IVec3::ZERO;
            normal[axis] = -step[axis];
            // Advance tied axes together: cells touched only along an edge are not occupied.
            for a in 0..3 {
                if (next[a] - distance).abs() < 1e-12 {
                    cell[a] += step[a];
                    next[a] += delta[a];
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn world() -> BlockWorld {
        BlockWorld::new(
            Arc::new(crate::state_registry::fixture()),
            IVec3::splat(-32),
            64,
        )
        .unwrap()
    }
    #[test]
    fn only_boundary_neighbors_are_invalidated() {
        let mut w = world();
        w.set(IVec3::new(-1, 5, 5), 1).unwrap();
        assert_eq!(w.take_dirty(), [[-1, 0, 0], [0, 0, 0]].into());
        w.set(IVec3::new(-1, 5, 5), 1).unwrap();
        assert!(w.take_dirty().is_empty());
        assert!(w.set(IVec3::ZERO, 999).is_err());
    }
    #[test]
    fn negative_axis_parallel_rays() {
        let mut w = world();
        w.set(IVec3::new(-2, 0, 0), 1).unwrap();
        let h = w.raycast(DVec3::new(2.5, 0.5, 0.5), -DVec3::X, 5.).unwrap();
        assert_eq!(h.cell, IVec3::new(-2, 0, 0));
        assert_eq!(h.normal, IVec3::X);
        assert!((h.distance - 3.5).abs() < 1e-10);
        assert!(w
            .raycast(DVec3::new(2.5, 0.5, 0.5), -DVec3::X, 3.)
            .is_none());
        assert!(w.raycast(DVec3::ZERO, DVec3::ZERO, 5.).is_none());
    }
    #[test]
    fn edge_touches_do_not_hit_neighbor() {
        let mut w = world();
        w.set(IVec3::new(1, 0, 0), 1).unwrap();
        w.set(IVec3::new(1, 1, 0), 70001).unwrap();
        assert_eq!(
            w.raycast(DVec3::new(0.5, 0.5, 0.5), DVec3::new(1., 1., 0.), 4.)
                .unwrap()
                .state,
            70001
        );
    }
    #[test]
    fn save_roundtrip_and_registry_binding() {
        let mut w = world();
        w.set(IVec3::new(1, 2, 3), 70001).unwrap();
        let json = w.to_json().unwrap();
        let loaded = BlockWorld::from_json(w.registry.clone(), &json).unwrap();
        assert_eq!(loaded.get(IVec3::new(1, 2, 3)), Some(70001));
        assert_eq!(loaded.active_chunks(10).unwrap(), [[0, 0, 0]].into());
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["registry_fingerprint"] = serde_json::json!("wrong");
        assert!(BlockWorld::from_json(w.registry.clone(), &v.to_string()).is_err());
        assert!(loaded.active_chunks(0).is_err());
    }
}
