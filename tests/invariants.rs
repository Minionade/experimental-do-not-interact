use glam::{DVec3, IVec3};
use orbital_voxel::{
    mesh,
    mods::Registry,
    octree::{BlockOctree, Bounds, NodeKey, CHUNK_VOLUME},
    physics::{example_ship, Physics, FIXED_DT},
    planet::{lod_depth, Planet, TerrainOctree, Walker},
};
fn planet() -> Planet {
    Planet::new(DVec3::ZERO, 96., 9.81)
}
#[test]
fn chunks_are_cubic() {
    assert_eq!(CHUNK_VOLUME, 4096);
}
#[test]
fn every_octant_is_addressable() {
    let mut tree = BlockOctree::new(IVec3::splat(-32), 64).unwrap();
    for z in [-31, 31] {
        for y in [-31, 31] {
            for x in [-31, 31] {
                let p = IVec3::new(x, y, z);
                let id = ((x > 0) as u32 + 2 * (y > 0) as u32 + 4 * (z > 0) as u32) + 1;
                tree.set(p, id).unwrap();
                assert_eq!(tree.get(p), Some(id));
            }
        }
    }
    assert_eq!(tree.get(IVec3::ZERO), Some(0));
    assert!(tree.get(IVec3::splat(32)).is_none());
}
#[test]
fn reverting_edits_compresses_tree() {
    let mut tree = BlockOctree::new(IVec3::splat(-512), 1024).unwrap();
    for p in [IVec3::new(411, -233, 91), IVec3::new(-311, 391, -401)] {
        tree.set(p, 3).unwrap();
    }
    assert!(tree.node_count() > 1);
    for p in [IVec3::new(411, -233, 91), IVec3::new(-311, 391, -401)] {
        tree.set(p, 0).unwrap();
    }
    assert_eq!(tree.node_count(), 1);
}
#[test]
fn octree_matches_dense_reference() {
    let mut t = BlockOctree::new(IVec3::ZERO, 32).unwrap();
    let mut dense = vec![0u32; 32 * 32 * 32];
    let mut rng = 17u32;
    for _ in 0..10000 {
        rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
        let x = (rng & 31) as i32;
        let y = ((rng >> 5) & 31) as i32;
        let z = ((rng >> 10) & 31) as i32;
        let id = (rng >> 15) & 3;
        t.set(IVec3::new(x, y, z), id).unwrap();
        dense[(x + 32 * (y + 32 * z)) as usize] = id;
    }
    for z in 0..32 {
        for y in 0..32 {
            for x in 0..32 {
                assert_eq!(
                    t.get(IVec3::new(x, y, z)),
                    Some(dense[(x + 32 * (y + 32 * z)) as usize])
                );
            }
        }
    }
}
#[test]
fn octree_save_roundtrip() {
    let t = example_ship();
    let loaded = BlockOctree::from_json(&t.to_json().unwrap()).unwrap();
    assert_eq!(t.to_json().unwrap(), loaded.to_json().unwrap());
}
#[test]
fn malformed_brick_is_rejected() {
    let json = r#"{"min":[0,0,0],"size":16,"root":{"Brick":{"cells":[1]}}}"#;
    assert!(BlockOctree::from_json(json).is_err());
}
#[test]
fn node_keys_roundtrip() {
    let root = NodeKey::ROOT;
    for i in 0..8 {
        assert_eq!(root.child(i).parent(), Some(root));
        for j in 0..8 {
            assert_eq!(root.child(i).child(j).parent(), Some(root.child(i)));
        }
    }
}
#[test]
fn child_bounds_tile_parent() {
    let b = Bounds {
        min: DVec3::splat(-4.),
        size: 8.,
    };
    let mut total = 0.;
    for i in 0..8 {
        let c = b.child(i);
        assert!(c.min.cmpge(b.min).all());
        assert!((c.min + DVec3::splat(c.size))
            .cmple(b.min + DVec3::splat(b.size))
            .all());
        total += c.size.powi(3);
    }
    assert_eq!(total, b.size.powi(3));
}
#[test]
fn circumnavigation_has_no_seam() {
    let p = planet();
    let mut w = Walker::new(&p);
    let start = w.clone();
    for _ in 0..8192 {
        w.walk(&p, std::f64::consts::TAU * p.radius / 8192., 0.);
        assert!((w.position.length() - (p.radius + w.eye_height)).abs() < 1e-8);
        assert!(w.forward.dot(p.up(w.position)).abs() < 1e-10);
    }
    assert!(w.position.distance(start.position) < 1e-8);
    assert!(w.forward.distance(start.forward) < 1e-10);
}
#[test]
fn strafing_crosses_poles_without_nan() {
    let p = planet();
    let mut w = Walker::new(&p);
    for _ in 0..5000 {
        w.yaw(&p, 0.001);
        w.walk(&p, 0.2, 0.3);
        assert!(w.position.is_finite() && w.forward.is_finite());
        assert!((w.forward.length() - 1.).abs() < 1e-10);
    }
}
#[test]
fn gravity_is_continuous_and_inverse_square() {
    let p = planet();
    assert_eq!(p.gravity(DVec3::ZERO), DVec3::ZERO);
    assert!((p.gravity(DVec3::X * 96.).length() - 9.81).abs() < 1e-10);
    assert!((p.gravity(DVec3::X * 192.).length() - 9.81 / 4.).abs() < 1e-10);
    assert!(
        p.gravity(DVec3::X * (96. - 1e-6))
            .distance(p.gravity(DVec3::X * (96. + 1e-6)))
            < 1e-5
    );
}
#[test]
fn distant_world_preserves_local_geometry() {
    let p = Planet::new(DVec3::splat(1e10), 96., 9.81);
    assert!((p.sdf(p.center + DVec3::X * 97.) - 1.).abs() < 1e-8);
    let local = (p.center + DVec3::X * 0.125 - p.center).as_vec3();
    assert_eq!(local.x, 0.125);
}
#[test]
fn lod_has_hysteresis() {
    let p = planet();
    assert_eq!(lod_depth(&p, DVec3::Y * p.radius * 2.3, 3), 3);
    assert_eq!(lod_depth(&p, DVec3::Y * p.radius * 2.3, 2), 2);
    assert_eq!(lod_depth(&p, DVec3::Y * p.radius * 3., 3), 2);
}
#[test]
fn surface_mesh_is_outward_and_finite() {
    let p = planet();
    let tree = TerrainOctree::generate(&p, 1);
    let meshes = mesh::mesh_terrain(&tree, &p);
    assert!(!meshes.is_empty());
    for mesh in meshes {
        for t in mesh.vertices.as_chunks::<3>().0 {
            let a = DVec3::from_array(t[0].position.map(|x| x as f64)) + mesh.bounds.min;
            let b = DVec3::from_array(t[1].position.map(|x| x as f64)) + mesh.bounds.min;
            let c = DVec3::from_array(t[2].position.map(|x| x as f64)) + mesh.bounds.min;
            assert!((b - a).cross(c - a).dot(a) > 0.);
            assert!(p.sdf(a).abs() < 1.0);
        }
    }
}
#[test]
fn neighboring_density_samples_agree() {
    let tree = TerrainOctree::generate(&planet(), 2);
    let chunks = tree.chunks();
    let a = chunks[0];
    let b = chunks
        .iter()
        .find(|b| b.bounds.min == a.bounds.min + DVec3::X * a.bounds.size)
        .unwrap();
    for y in 0..17 {
        for z in 0..17 {
            assert_eq!(a.sample(16, y, z), b.sample(0, y, z));
        }
    }
}
#[test]
fn face_culling_removes_internal_faces() {
    let mut t = BlockOctree::new(IVec3::ZERO, 16).unwrap();
    t.set(IVec3::ZERO, 1).unwrap();
    t.set(IVec3::X, 1).unwrap();
    let m = mesh::mesh_blocks(&t, IVec3::ZERO, IVec3::splat(2));
    assert_eq!(m.vertices.len(), 10 * 6);
}
#[test]
fn orbit_remains_bounded() {
    let p = planet();
    let r = 144.;
    let speed = p.circular_speed(r - p.radius);
    let mut s = Physics::new(vec![p]);
    let h = s.spawn_ball(DVec3::X * r, DVec3::Y * speed, 0.2);
    let steps = (std::f64::consts::TAU * r / speed / FIXED_DT) as usize;
    let mut max_drift = 0f64;
    for _ in 0..steps {
        s.step(None);
        max_drift = max_drift.max((s.pose(h).0.length() - r).abs());
    }
    assert!(max_drift < 0.3, "drift {max_drift}");
}
#[test]
fn falling_body_collides_with_planet() {
    let p = planet();
    let mut s = Physics::new(vec![p]);
    let h = s.spawn_ball(DVec3::Y * 101., DVec3::ZERO, 0.5);
    for _ in 0..1200 {
        s.step(None);
    }
    let y = s.pose(h).0.y;
    assert!((96.45..96.6).contains(&y), "rest height {y}");
}
#[test]
fn compound_ship_collides_and_thruster_lifts() {
    let mut s = Physics::new(vec![planet()]);
    let t = example_ship();
    let h = s
        .spawn_structure(&t, IVec3::splat(-4), IVec3::splat(5), DVec3::Y * 101.)
        .unwrap();
    for _ in 0..600 {
        s.step(None);
    }
    assert!(s.pose(h).0.length() > 95.);
    let before = s.pose(h).0;
    for _ in 0..240 {
        s.step(Some((h, DVec3::Y * 30.)));
    }
    assert!(s.pose(h).0.y > before.y + 20.);
}
#[test]
fn frame_partition_does_not_change_fixed_steps() {
    let mut a = Physics::new(vec![]);
    let mut b = Physics::new(vec![]);
    let ha = a.spawn_ball(DVec3::ZERO, DVec3::X, 0.2);
    let hb = b.spawn_ball(DVec3::ZERO, DVec3::X, 0.2);
    for _ in 0..120 {
        a.advance(1. / 120., None);
    }
    for _ in 0..60 {
        b.advance(1. / 60., None);
    }
    assert!(a.pose(ha).0.distance(b.pose(hb).0) < 1e-12);
}
#[test]
fn registry_load_is_atomic() {
    let mut r = Registry::default();
    r.load_toml(include_str!("../examples/mods/example.toml"))
        .unwrap();
    let before = r.len();
    assert!(r.load_toml("api=1\nid='newmod'\n[[blocks]]\nid='newmod:ok'\ndensity=3\ncolor=[0.2,0.2,0.2]\n[[blocks]]\nid='other:no'\ndensity=3\ncolor=[0.2,0.2,0.2]").is_err());
    assert_eq!(r.len(), before);
    assert!(r.get("newmod:ok").is_none());
}

#[test]
fn native_mod_commands_run_in_the_physics_tick() {
    use orbital_voxel::mods::{Command, Event, NativeMod};
    struct Booster(Option<orbital_voxel::physics::BodyHandle>);
    impl NativeMod for Booster {
        fn on_event(&mut self, event: &Event, out: &mut Vec<Command>) {
            match event {
                Event::StructureSpawned(body) => self.0 = Some(*body),
                Event::FixedTick { .. } => {
                    if let Some(body) = self.0 {
                        out.push(Command::Accelerate {
                            body,
                            acceleration: DVec3::Y * 5.,
                        });
                    }
                }
            }
        }
    }
    let mut world = Physics::new(vec![]);
    world.mods.register(Booster(None));
    let h = world
        .spawn_structure(
            &example_ship(),
            IVec3::splat(-4),
            IVec3::splat(5),
            DVec3::ZERO,
        )
        .unwrap();
    for _ in 0..120 {
        world.step(None);
    }
    assert_eq!(world.tick, 120);
    assert!((world.velocity(h).y - 5.).abs() < 1e-8);
    assert!((world.pose(h).0.y - 2.5).abs() < 1e-8);
}
#[test]
fn terrain_mesh_is_watertight_across_chunks() {
    use std::collections::HashMap;
    let p = planet();
    let tree = TerrainOctree::generate(&p, 1);
    let mut edges = HashMap::new();
    for mesh in mesh::mesh_terrain(&tree, &p) {
        for triangle in mesh.vertices.as_chunks::<3>().0 {
            let points: Vec<[i64; 3]> = triangle
                .iter()
                .map(|v| {
                    let point = DVec3::from_array(v.position.map(|x| x as f64)) + mesh.bounds.min;
                    point.to_array().map(|x| (x * 10000.).round() as i64)
                })
                .collect();
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                if points[a] == points[b] {
                    continue;
                }
                let edge = if points[a] < points[b] {
                    (points[a], points[b])
                } else {
                    (points[b], points[a])
                };
                *edges.entry(edge).or_insert(0) += 1;
            }
        }
    }
    assert!(
        edges.values().all(|n| *n == 2),
        "{} unpaired/nonmanifold edges",
        edges.values().filter(|n| **n != 2).count()
    );
}

#[test]
fn octree_preserves_large_vanilla_state_ids() {
    let mut tree = BlockOctree::new(IVec3::ZERO, 16).unwrap();
    tree.set(IVec3::ZERO, 90001).unwrap();
    let loaded = BlockOctree::from_json(&tree.to_json().unwrap()).unwrap();
    assert_eq!(loaded.get(IVec3::ZERO), Some(90001));
}
