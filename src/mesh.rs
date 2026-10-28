use crate::{
    octree::{BlockOctree, Bounds, NodeKey, AIR, CHUNK_EDGE},
    planet::{Planet, TerrainChunk, TerrainOctree},
};
use bytemuck::{Pod, Zeroable};
use glam::{DVec3, IVec3};
use rayon::prelude::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}
#[derive(Clone, Debug)]
pub struct Mesh {
    pub key: NodeKey,
    pub bounds: Bounds,
    pub vertices: Vec<Vertex>,
}

/// Parallel leaf meshing. Coarse solid/empty octree nodes allocate no meshes.
pub fn mesh_terrain(tree: &TerrainOctree, planet: &Planet) -> Vec<Mesh> {
    tree.chunks()
        .par_iter()
        .map(|c| mesh_chunk(c, planet))
        .filter(|m| !m.vertices.is_empty())
        .collect()
}
fn mesh_chunk(c: &TerrainChunk, p: &Planet) -> Mesh {
    let step = c.bounds.size / CHUNK_EDGE as f64;
    let mut vertices = Vec::new();
    const TETS: [[usize; 4]; 6] = [
        [0, 1, 3, 7],
        [0, 3, 2, 7],
        [0, 2, 6, 7],
        [0, 6, 4, 7],
        [0, 4, 5, 7],
        [0, 5, 1, 7],
    ];
    for z in 0..CHUNK_EDGE {
        for y in 0..CHUNK_EDGE {
            for x in 0..CHUNK_EDGE {
                let positions: [DVec3; 8] = std::array::from_fn(|i| {
                    c.bounds.min
                        + DVec3::new(
                            (x + (i & 1)) as f64,
                            (y + ((i >> 1) & 1)) as f64,
                            (z + ((i >> 2) & 1)) as f64,
                        ) * step
                });
                let d: [f32; 8] = std::array::from_fn(|i| {
                    c.sample(x + (i & 1), y + ((i >> 1) & 1), z + ((i >> 2) & 1))
                });
                if d.iter().all(|v| *v >= 0.0) || d.iter().all(|v| *v < 0.0) {
                    continue;
                }
                for tet in TETS {
                    let mut inside = [0; 4];
                    let mut outside = [0; 4];
                    let (mut ni, mut no) = (0, 0);
                    for i in tet {
                        if d[i] < 0.0 {
                            inside[ni] = i;
                            ni += 1;
                        } else {
                            outside[no] = i;
                            no += 1;
                        }
                    }
                    let edge = |a: usize, b: usize| {
                        positions[a].lerp(positions[b], d[a] as f64 / (d[a] as f64 - d[b] as f64))
                    };
                    match ni {
                        1 => emit(
                            [
                                edge(inside[0], outside[0]),
                                edge(inside[0], outside[1]),
                                edge(inside[0], outside[2]),
                            ],
                            c.bounds.min,
                            p,
                            &mut vertices,
                        ),
                        3 => emit(
                            [
                                edge(outside[0], inside[0]),
                                edge(outside[0], inside[1]),
                                edge(outside[0], inside[2]),
                            ],
                            c.bounds.min,
                            p,
                            &mut vertices,
                        ),
                        2 => {
                            let a = edge(inside[0], outside[0]);
                            let b = edge(inside[0], outside[1]);
                            let e = edge(inside[1], outside[0]);
                            let f = edge(inside[1], outside[1]);
                            emit([a, b, e], c.bounds.min, p, &mut vertices);
                            emit([b, f, e], c.bounds.min, p, &mut vertices);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Mesh {
        key: c.key,
        bounds: c.bounds,
        vertices,
    }
}
fn emit(mut tri: [DVec3; 3], origin: DVec3, planet: &Planet, out: &mut Vec<Vertex>) {
    let cross = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
    if cross.length_squared() < 1e-18 {
        return;
    }
    if cross.dot(tri[0] - planet.center) < 0.0 {
        tri.swap(1, 2);
    }
    for v in tri {
        let normal = planet.up(v);
        // Original procedural colors, no Minecraft assets.
        let continental = (normal.x * 5.0 + normal.z * 2.0).sin()
            + (normal.z * 7.0 - normal.y * 3.0).cos() * 0.65;
        let color = if planet.radius < 60.0 {
            [0.53, 0.36, 0.25]
        } else if normal.y.abs() > 0.90 {
            [0.78, 0.87, 0.89]
        } else if continental < -0.1 {
            [0.045, 0.23, 0.38]
        } else {
            [0.19 + continental as f32 * 0.025, 0.42, 0.22]
        };
        out.push(Vertex {
            position: (v - origin).as_vec3().to_array(),
            normal: normal.as_vec3().to_array(),
            color,
        });
    }
}

/// Face-culling mesher for orthogonal block structures. Greedy merging is future work.
pub fn mesh_blocks(tree: &BlockOctree, min: IVec3, max: IVec3) -> Mesh {
    let mut vertices = Vec::new();
    let faces = [
        (IVec3::X, [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]]),
        (-IVec3::X, [[0, 0, 1], [0, 1, 1], [0, 1, 0], [0, 0, 0]]),
        (IVec3::Y, [[0, 1, 1], [1, 1, 1], [1, 1, 0], [0, 1, 0]]),
        (-IVec3::Y, [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]]),
        (IVec3::Z, [[1, 0, 1], [1, 1, 1], [0, 1, 1], [0, 0, 1]]),
        (-IVec3::Z, [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]]),
    ];
    for z in min.z..max.z {
        for y in min.y..max.y {
            for x in min.x..max.x {
                let pos = IVec3::new(x, y, z);
                let id = tree.get(pos).unwrap_or(AIR);
                if id == AIR {
                    continue;
                }
                for (normal, corners) in faces {
                    if tree.get(pos + normal).unwrap_or(AIR) != AIR {
                        continue;
                    }
                    for index in [0, 1, 2, 0, 2, 3] {
                        let v = pos.as_vec3()
                            + glam::Vec3::from_array(corners[index].map(|x| x as f32));
                        vertices.push(Vertex {
                            position: v.to_array(),
                            normal: normal.as_vec3().to_array(),
                            color: if id == 2 {
                                [0.95, 0.52, 0.12]
                            } else {
                                [0.59, 0.67, 0.72]
                            },
                        });
                    }
                }
            }
        }
    }
    Mesh {
        key: NodeKey::ROOT,
        bounds: Bounds {
            min: min.as_dvec3(),
            size: (max - min).max_element() as f64,
        },
        vertices,
    }
}
