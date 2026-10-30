//! Broad-phase queries over cubic octree cells and authoritative static shapes.
use crate::{
    block_world::{BlockWorld, Hit},
    collision::Aabb,
    oracle::PhysicsCatalog,
};
use glam::{DVec3, IVec3};
fn cells(bounds: Aabb) -> Result<Vec<IVec3>, String> {
    Aabb::new(bounds.min, bounds.max)?;
    if bounds.min.min_element() < i32::MIN as f64 + 2.0
        || bounds.max.max_element() > i32::MAX as f64 - 2.0
    {
        return Err("collision query outside coordinate range".into());
    }
    // The pinned shape report contains static extents between -0.25 and 1.5.
    // One neighboring cell on each side covers these overhangs.
    let min = bounds.min.floor().as_ivec3() - IVec3::ONE;
    let max = bounds.max.floor().as_ivec3() + IVec3::ONE;
    if (max - min).max_element() > 64 {
        return Err("collision query exceeds local budget".into());
    }
    let mut result = Vec::new();
    for z in min.z..=max.z {
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                result.push(IVec3::new(x, y, z));
            }
        }
    }
    Ok(result)
}
pub fn colliders(
    world: &BlockWorld,
    physics: &PhysicsCatalog,
    bounds: Aabb,
) -> Result<Vec<Aabb>, String> {
    let mut result = Vec::new();
    for cell in cells(bounds)? {
        let Some(id) = world.get(cell) else {
            continue;
        };
        if world.registry.is_air(id) {
            continue;
        }
        for shape in physics.static_boxes(id, false)? {
            let shape = shape.translated(cell.as_dvec3());
            if bounds.intersects(shape) {
                result.push(shape);
            }
        }
    }
    Ok(result)
}
fn ray_box(origin: DVec3, direction: DVec3, shape: Aabb, reach: f64) -> Option<(f64, IVec3)> {
    let mut near: f64 = 0.0;
    let mut far = reach;
    let mut normal = IVec3::ZERO;
    for axis in 0..3 {
        if direction[axis] == 0.0 {
            if origin[axis] < shape.min[axis] || origin[axis] >= shape.max[axis] {
                return None;
            }
            continue;
        }
        let a = (shape.min[axis] - origin[axis]) / direction[axis];
        let b = (shape.max[axis] - origin[axis]) / direction[axis];
        let entry = a.min(b);
        if entry > near {
            near = entry;
            normal = IVec3::ZERO;
            normal[axis] = if direction[axis] > 0.0 { -1 } else { 1 };
        }
        far = far.min(a.max(b));
        if far <= near {
            return None;
        }
    }
    Some((near, normal))
}
pub fn raycast(
    world: &BlockWorld,
    physics: &PhysicsCatalog,
    origin: DVec3,
    direction: DVec3,
    reach: f64,
) -> Result<Option<Hit>, String> {
    if !origin.is_finite()
        || !direction.is_finite()
        || direction.length_squared() < 1e-24
        || !reach.is_finite()
        || !(0.0..=16.0).contains(&reach)
    {
        return Err("invalid selection ray".into());
    }
    if reach == 0.0 {
        return Ok(None);
    }
    let direction = direction.normalize();
    let end = origin + direction * reach;
    let bounds = Aabb::new(
        origin.min(end) - DVec3::splat(1e-8),
        origin.max(end) + DVec3::splat(1e-8),
    )?;
    let mut hit: Option<Hit> = None;
    for cell in cells(bounds)? {
        let Some(id) = world.get(cell) else {
            continue;
        };
        if world.registry.is_air(id) {
            continue;
        }
        for shape in physics.static_boxes(id, true)? {
            if let Some((distance, normal)) =
                ray_box(origin, direction, shape.translated(cell.as_dvec3()), reach)
            {
                if hit.as_ref().is_none_or(|h| distance < h.distance) {
                    hit = Some(Hit {
                        cell,
                        state: id,
                        distance,
                        normal,
                    });
                }
            }
        }
    }
    Ok(hit)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ray_passes_above_slab() {
        let slab = Aabb::new(DVec3::ZERO, DVec3::new(1.0, 0.5, 1.0)).unwrap();
        assert!(ray_box(DVec3::new(-2.0, 0.75, 0.5), DVec3::X, slab, 6.0).is_none());
        assert_eq!(
            ray_box(DVec3::new(-2.0, 0.25, 0.5), DVec3::X, slab, 6.0),
            Some((2.0, -IVec3::X))
        );
    }
    #[test]
    fn ray_rejects_tangent_and_out_of_reach() {
        let shape = Aabb::new(DVec3::ZERO, DVec3::ONE).unwrap();
        assert!(ray_box(DVec3::new(-2.0, 1.0, 0.5), DVec3::X, shape, 6.0).is_none());
        assert!(ray_box(DVec3::new(-2.0, 0.5, 0.5), DVec3::X, shape, 1.0).is_none());
    }
}
