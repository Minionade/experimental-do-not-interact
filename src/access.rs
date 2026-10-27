//! World-coordinate block access shared by lighting, fluids, entity physics and the game layer.
use crate::chunk::{BlockState, Chunk, HEIGHT, MIN_Y};
use std::collections::HashMap;
use std::sync::Arc;

pub type ChunkMap = HashMap<(i32, i32), Chunk>;
pub type SharedChunkMap = HashMap<(i32, i32), Arc<Chunk>>;

pub trait BlockAccess {
    /// None when the position is outside loaded chunks or the height limits.
    fn block(&self, x: i32, y: i32, z: i32) -> Option<&BlockState>;
    /// Returns false if the position is not loaded.
    fn set_block(&mut self, x: i32, y: i32, z: i32, b: &BlockState) -> bool;
}

/// Anything that stores chunks by chunk coordinates. Gets `BlockAccess` for free.
pub trait ChunkStore {
    fn chunk(&self, cx: i32, cz: i32) -> Option<&Chunk>;
    fn chunk_mut(&mut self, cx: i32, cz: i32) -> Option<&mut Chunk>;
}

impl ChunkStore for ChunkMap {
    fn chunk(&self, cx: i32, cz: i32) -> Option<&Chunk> { self.get(&(cx, cz)) }
    fn chunk_mut(&mut self, cx: i32, cz: i32) -> Option<&mut Chunk> { self.get_mut(&(cx, cz)) }
}

/// Copy-on-write: mutating a chunk shared with a worker thread clones it first.
impl ChunkStore for SharedChunkMap {
    fn chunk(&self, cx: i32, cz: i32) -> Option<&Chunk> { self.get(&(cx, cz)).map(|c| &**c) }
    fn chunk_mut(&mut self, cx: i32, cz: i32) -> Option<&mut Chunk> { self.get_mut(&(cx, cz)).map(Arc::make_mut) }
}

fn in_height(y: i32) -> bool { (MIN_Y..MIN_Y + HEIGHT as i32).contains(&y) }

impl<T: ChunkStore> BlockAccess for T {
    fn block(&self, x: i32, y: i32, z: i32) -> Option<&BlockState> {
        if !in_height(y) { return None; }
        self.chunk(x >> 4, z >> 4).map(|c| c.get((x & 15) as usize, y, (z & 15) as usize))
    }
    fn set_block(&mut self, x: i32, y: i32, z: i32, b: &BlockState) -> bool {
        if !in_height(y) { return false; }
        match self.chunk_mut(x >> 4, z >> 4) {
            Some(c) => { c.set((x & 15) as usize, y, (z & 15) as usize, b); true }
            None => false,
        }
    }
}
