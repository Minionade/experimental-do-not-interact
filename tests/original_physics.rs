//! Run explicitly with MINECRAFT_REPORTS pointing to generated pinned reports.
use glam::{DVec3, IVec3};
use orbital_voxel::{
    block_collision,
    block_world::BlockWorld,
    collision::{self, Aabb},
    oracle::PhysicsCatalog,
    state_registry::StateRegistry,
    vanilla::RegistryReports,
};
use std::{path::PathBuf, sync::Arc};
#[test]
#[ignore = "requires original hash-pinned runtime reports"]
fn original_shapes_drive_selection_landing_and_steps() {
    let path = PathBuf::from(std::env::var("MINECRAFT_REPORTS").expect("set MINECRAFT_REPORTS"));
    let reports = RegistryReports::load_pinned(&path).unwrap();
    let registry = Arc::new(StateRegistry::from_reports(&reports).unwrap());
    let physics = PhysicsCatalog::load_pinned(&path.join("physics.json"), &registry).unwrap();
    let mut world = BlockWorld::new(registry.clone(), IVec3::splat(-32), 64).unwrap();
    let slab = registry.resolve("stone_slab", &Default::default()).unwrap();
    let stone = registry.resolve("stone", &Default::default()).unwrap();
    world.set(IVec3::ZERO, slab).unwrap();
    world.set(IVec3::new(2, 0, 0), stone).unwrap();
    let hit =
        block_collision::raycast(&world, &physics, DVec3::new(-2.0, 0.75, 0.5), DVec3::X, 6.0)
            .unwrap()
            .unwrap();
    assert_eq!(hit.cell, IVec3::new(2, 0, 0));
    let hit =
        block_collision::raycast(&world, &physics, DVec3::new(-2.0, 0.25, 0.5), DVec3::X, 6.0)
            .unwrap()
            .unwrap();
    assert_eq!(hit.cell, IVec3::ZERO);
    let body = Aabb::new(DVec3::new(0.2, 10.0, 0.2), DVec3::new(0.8, 11.8, 0.8)).unwrap();
    let desired = -DVec3::Y * 20.0;
    let boxes =
        block_collision::colliders(&world, &physics, body.swept_bounds(desired, 0.0)).unwrap();
    let landed = collision::move_body(body, &boxes, desired, 0.0, false).unwrap();
    assert_eq!(landed.delta.y, -9.5);
    assert!(landed.grounded);
    let body = Aabb::new(DVec3::new(-0.8, 0.0, 0.2), DVec3::new(-0.2, 1.8, 0.8)).unwrap();
    let desired = DVec3::new(0.8, -0.1, 0.0);
    let boxes =
        block_collision::colliders(&world, &physics, body.swept_bounds(desired, 0.6)).unwrap();
    let step = collision::move_body(body, &boxes, desired, 0.6, true).unwrap();
    assert!((step.delta.y - 0.5).abs() < 1e-12);
    assert_eq!(step.delta.x, 0.8);
    let fence = registry.resolve("oak_fence", &Default::default()).unwrap();
    assert_eq!(
        physics
            .static_boxes(fence, false)
            .unwrap()
            .iter()
            .map(|b| b.max.y)
            .fold(0.0, f64::max),
        1.5
    );
    let powder_snow = registry
        .resolve("powder_snow", &Default::default())
        .unwrap();
    assert!(physics.static_boxes(powder_snow, false).is_err());
    let flower = registry.resolve("dandelion", &Default::default()).unwrap();
    assert!(physics.static_boxes(flower, true).is_err());
}
