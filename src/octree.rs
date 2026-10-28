//! Sparse, cubic, three-dimensional storage. No vertical columns.
use glam::{DVec3, IVec3};
use serde::{Deserialize, Serialize};

pub const CHUNK_EDGE: usize = 16;
pub const CHUNK_VOLUME: usize = CHUNK_EDGE * CHUNK_EDGE * CHUNK_EDGE;
pub type BlockId = u16;
pub const AIR: BlockId = 0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: DVec3,
    pub size: f64,
}
impl Bounds {
    pub fn center(self) -> DVec3 {
        self.min + DVec3::splat(self.size * 0.5)
    }
    pub fn child(self, index: usize) -> Self {
        let size = self.size * 0.5;
        let offset = DVec3::new(
            (index & 1) as f64,
            ((index >> 1) & 1) as f64,
            ((index >> 2) & 1) as f64,
        );
        Self {
            min: self.min + offset * size,
            size,
        }
    }
    pub fn distance(self, p: DVec3) -> f64 {
        (p - p.clamp(self.min, self.min + DVec3::splat(self.size))).length()
    }
}

/// A 16^3 brick. Index order is X, then Y, then Z; all axes are equal.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Brick {
    cells: Vec<BlockId>,
}
impl Brick {
    pub fn filled(value: BlockId) -> Self {
        Self {
            cells: vec![value; CHUNK_VOLUME],
        }
    }
    pub fn get(&self, p: IVec3) -> BlockId {
        self.cells[index(p)]
    }
    pub fn set(&mut self, p: IVec3, value: BlockId) {
        self.cells[index(p)] = value;
    }
    pub fn uniform(&self) -> Option<BlockId> {
        let first = self.cells[0];
        self.cells.iter().all(|v| *v == first).then_some(first)
    }
    fn valid(&self) -> bool {
        self.cells.len() == CHUNK_VOLUME
    }
}
fn index(p: IVec3) -> usize {
    assert!(p.cmpge(IVec3::ZERO).all() && p.cmplt(IVec3::splat(16)).all());
    (p.x + 16 * (p.y + 16 * p.z)) as usize
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Node {
    Uniform(BlockId),
    Branch(Box<[Node; 8]>),
    Brick(Brick),
}
impl Node {
    fn read(&self, p: IVec3, size: i32) -> BlockId {
        match self {
            Self::Uniform(v) => *v,
            Self::Brick(brick) => brick.get(p),
            Self::Branch(children) => {
                let (i, local) = octant(p, size / 2);
                children[i].read(local, size / 2)
            }
        }
    }
    fn write(&mut self, p: IVec3, size: i32, value: BlockId) {
        if let Self::Uniform(old) = self {
            if *old == value {
                return;
            }
            *self = if size == 16 {
                Self::Brick(Brick::filled(*old))
            } else {
                Self::Branch(Box::new(std::array::from_fn(|_| Self::Uniform(*old))))
            };
        }
        match self {
            Self::Brick(b) => {
                b.set(p, value);
                if let Some(v) = b.uniform() {
                    *self = Self::Uniform(v);
                }
            }
            Self::Branch(c) => {
                let (i, local) = octant(p, size / 2);
                c[i].write(local, size / 2, value);
                if let Self::Uniform(v) = c[0] {
                    if c.iter().all(|n| matches!(n, Self::Uniform(x) if *x == v)) {
                        *self = Self::Uniform(v);
                    }
                }
            }
            Self::Uniform(_) => unreachable!(),
        }
    }
    fn count(&self) -> usize {
        match self {
            Self::Branch(c) => 1 + c.iter().map(Self::count).sum::<usize>(),
            _ => 1,
        }
    }
    fn validate(&self, size: i32) -> bool {
        match self {
            Self::Uniform(_) => true,
            Self::Brick(b) => size == 16 && b.valid(),
            Self::Branch(c) => size > 16 && c.iter().all(|n| n.validate(size / 2)),
        }
    }
}
fn octant(p: IVec3, half: i32) -> (usize, IVec3) {
    let high = IVec3::new(
        (p.x >= half) as i32,
        (p.y >= half) as i32,
        (p.z >= half) as i32,
    );
    (
        (high.x | high.y << 1 | high.z << 2) as usize,
        p - high * half,
    )
}

/// Root is aligned to `min`, with power-of-two extent. Empty space occupies one node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockOctree {
    min: IVec3,
    size: i32,
    root: Node,
}
impl BlockOctree {
    pub fn new(min: IVec3, size: i32) -> Result<Self, String> {
        if !(16..=1_048_576).contains(&size) || !(size as u32).is_power_of_two() {
            return Err("octree size must be a power of two in 16..=1048576".into());
        }
        if min.x.checked_add(size).is_none()
            || min.y.checked_add(size).is_none()
            || min.z.checked_add(size).is_none()
        {
            return Err("octree bounds overflow".into());
        }
        Ok(Self {
            min,
            size,
            root: Node::Uniform(AIR),
        })
    }
    pub fn contains(&self, p: IVec3) -> bool {
        p.cmpge(self.min).all() && p.cmplt(self.min + IVec3::splat(self.size)).all()
    }
    pub fn get(&self, p: IVec3) -> Option<BlockId> {
        self.contains(p)
            .then(|| self.root.read(p - self.min, self.size))
    }
    pub fn set(&mut self, p: IVec3, value: BlockId) -> Result<(), String> {
        if !self.contains(p) {
            return Err(format!("cell {p} outside octree"));
        }
        self.root.write(p - self.min, self.size, value);
        Ok(())
    }
    pub fn node_count(&self) -> usize {
        self.root.count()
    }
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
    pub fn from_json(s: &str) -> Result<Self, String> {
        let tree: Self = serde_json::from_str(s).map_err(|e| e.to_string())?;
        Self::new(tree.min, tree.size)?;
        if !tree.root.validate(tree.size) {
            return Err("invalid octree shape or brick length".into());
        }
        Ok(tree)
    }
}

/// Parent occupies the high bits. Root = 1; each child appends three bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeKey(pub u64);
impl NodeKey {
    pub const ROOT: Self = Self(1);
    pub fn child(self, octant: usize) -> Self {
        assert!(octant < 8 && self.0 < 1 << 60);
        Self((self.0 << 3) | octant as u64)
    }
    pub fn parent(self) -> Option<Self> {
        (self != Self::ROOT).then_some(Self(self.0 >> 3))
    }
}
