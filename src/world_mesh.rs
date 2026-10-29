//! Original-model chunk meshes with conservative boundary occlusion.
use crate::{
    block_world::{BlockWorld, ChunkKey},
    blockstate,
    octree::AIR,
    sprite::{AlphaMode, Sprite},
    state_registry::StateRegistry,
    vanilla::{ClientAssets, ResourceId},
    vanilla_mesh::ModelQuad,
};
use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Vec3};
use std::collections::BTreeMap;
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TexturedVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub normal: [f32; 3],
}
pub struct ChunkMesh {
    pub key: ChunkKey,
    pub batches: BTreeMap<ResourceId, Vec<TexturedVertex>>,
}
pub struct CachedModel {
    pub quads: Vec<ModelQuad>,
    pub occludes: [bool; 6],
}
#[derive(Default)]
pub struct ModelCache {
    pub models: BTreeMap<u32, CachedModel>,
    pub sprites: BTreeMap<ResourceId, Sprite>,
}
pub const DIRECTIONS: [(&str, IVec3); 6] = [
    ("east", IVec3::X),
    ("west", IVec3::NEG_X),
    ("up", IVec3::Y),
    ("down", IVec3::NEG_Y),
    ("south", IVec3::Z),
    ("north", IVec3::NEG_Z),
];
impl ModelCache {
    pub fn ensure(
        &mut self,
        id: u32,
        registry: &StateRegistry,
        assets: &mut ClientAssets,
    ) -> Result<(), String> {
        if self.models.contains_key(&id) {
            return Ok(());
        }
        let state = registry
            .get(id)
            .ok_or_else(|| format!("unknown state {id}"))?;
        let mut quads = Vec::new();
        if !registry.is_air(id) {
            let d = assets.blockstate(&ResourceId::parse(&state.block)?)?;
            for choice in blockstate::select(&d, &state.properties, || 0)? {
                quads.extend(blockstate::bake_choice(assets, &choice)?);
            }
        }
        for q in &quads {
            if !self.sprites.contains_key(&q.texture.sprite) {
                self.sprites.insert(
                    q.texture.sprite.clone(),
                    Sprite::load(assets, &q.texture.sprite)?,
                );
            }
        }
        let mut occludes = [false; 6];
        for q in &quads {
            if self.sprites[&q.texture.sprite].alpha != AlphaMode::Opaque
                || q.texture.metadata["force_translucent"].as_bool() == Some(true)
            {
                continue;
            }
            for (face, (_, n)) in DIRECTIONS.iter().enumerate() {
                occludes[face] |= full_boundary(q, *n);
            }
        }
        self.models.insert(id, CachedModel { quads, occludes });
        Ok(())
    }
    pub fn prepare_chunk(
        &mut self,
        world: &BlockWorld,
        key: ChunkKey,
        assets: &mut ClientAssets,
    ) -> Result<(), String> {
        let min = IVec3::from_array(key) * 16;
        let mut ids = std::collections::BTreeSet::new();
        for z in -1..17 {
            for y in -1..17 {
                for x in -1..17 {
                    ids.insert(world.get(min + IVec3::new(x, y, z)).unwrap_or(AIR));
                }
            }
        }
        for id in ids {
            self.ensure(id, &world.registry, assets)?;
        }
        Ok(())
    }
}
fn full_boundary(q: &ModelQuad, n: IVec3) -> bool {
    if Vec3::from_array(q.normal).dot(n.as_vec3()) < 0.99999 {
        return false;
    }
    let axis = if n.x != 0 {
        0
    } else if n.y != 0 {
        1
    } else {
        2
    };
    let plane = if n[axis] > 0 { 1. } else { 0. };
    let mut corners = 0u8;
    for v in &q.vertices {
        if (v.position[axis] - plane).abs() > 1e-5 {
            return false;
        }
        let a = v.position[(axis + 1) % 3];
        let b = v.position[(axis + 2) % 3];
        if (a - a.round()).abs() > 1e-5
            || (b - b.round()).abs() > 1e-5
            || !(-1e-5..=1.00001).contains(&a)
            || !(-1e-5..=1.00001).contains(&b)
        {
            return false;
        }
        corners |= 1 << ((a.round() as u8) + 2 * (b.round() as u8));
    }
    corners == 15
}
pub fn mesh_chunk(
    world: &BlockWorld,
    key: ChunkKey,
    cache: &ModelCache,
) -> Result<ChunkMesh, String> {
    let min = IVec3::from_array(key) * 16;
    let mut batches: BTreeMap<ResourceId, Vec<TexturedVertex>> = BTreeMap::new();
    for z in 0..16 {
        for y in 0..16 {
            for x in 0..16 {
                let local = IVec3::new(x, y, z);
                let pos = min + local;
                let id = world.get(pos).unwrap_or(AIR);
                if world.registry.is_air(id) {
                    continue;
                }
                let model = cache.models.get(&id).ok_or("state model not prepared")?;
                for q in &model.quads {
                    if let Some(face) = &q.cull_face {
                        let side = DIRECTIONS
                            .iter()
                            .position(|(name, _)| name == face)
                            .ok_or("invalid cull direction")?;
                        let neighbor = world.get(pos + DIRECTIONS[side].1).unwrap_or(AIR);
                        if cache
                            .models
                            .get(&neighbor)
                            .ok_or("neighbor model not prepared")?
                            .occludes[side ^ 1]
                        {
                            continue;
                        }
                    }
                    let batch = batches.entry(q.texture.sprite.clone()).or_default();
                    for i in [0, 1, 2, 0, 2, 3] {
                        batch.push(TexturedVertex {
                            position: (Vec3::from_array(q.vertices[i].position) + local.as_vec3())
                                .to_array(),
                            uv: q.vertices[i].uv,
                            normal: q.normal,
                        });
                    }
                }
            }
        }
    }
    Ok(ChunkMesh { key, batches })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    fn cache() -> ModelCache {
        let mut faces = serde_json::Map::new();
        for (name, _) in DIRECTIONS {
            faces.insert(
                name.into(),
                json!({"texture":"block/stone","cullface":name}),
            );
        }
        let m = crate::vanilla::ResolvedModel {
            elements: Some(vec![json!({"from":[0,0,0],"to":[16,16,16],"faces":faces})]),
            ..Default::default()
        };
        ModelCache {
            models: [
                (
                    0,
                    CachedModel {
                        quads: vec![],
                        occludes: [false; 6],
                    },
                ),
                (
                    1,
                    CachedModel {
                        quads: crate::vanilla_mesh::bake_model(&m).unwrap(),
                        occludes: [true; 6],
                    },
                ),
            ]
            .into(),
            sprites: BTreeMap::new(),
        }
    }
    #[test]
    fn cross_chunk_faces_and_local_positions() {
        let mut w =
            BlockWorld::new(Arc::new(crate::state_registry::fixture()), IVec3::ZERO, 32).unwrap();
        w.set(IVec3::new(15, 1, 1), 1).unwrap();
        w.set(IVec3::new(16, 1, 1), 1).unwrap();
        let c = cache();
        let a = mesh_chunk(&w, [0, 0, 0], &c).unwrap();
        let b = mesh_chunk(&w, [1, 0, 0], &c).unwrap();
        assert_eq!(a.batches.values().map(Vec::len).sum::<usize>(), 30);
        assert_eq!(b.batches.values().map(Vec::len).sum::<usize>(), 30);
        assert!(b.batches.values().flatten().all(|v| v.position[0] <= 1.));
        w.set(IVec3::new(16, 1, 1), 0).unwrap();
        assert_eq!(
            mesh_chunk(&w, [0, 0, 0], &c)
                .unwrap()
                .batches
                .values()
                .map(Vec::len)
                .sum::<usize>(),
            36
        );
    }
    #[test]
    fn partial_faces_are_not_full_occluders() {
        let c = cache();
        let q = &c.models[&1].quads[0];
        assert!(DIRECTIONS.iter().any(|(_, n)| full_boundary(q, *n)));
        let mut q = q.clone();
        q.vertices[0].position[1] = 0.5;
        assert!(!DIRECTIONS.iter().any(|(_, n)| full_boundary(&q, *n)));
    }
}
