//! World-coordinate block access shared by lighting, fluids and entity physics.
use crate::chunk::{BlockState, Chunk, HEIGHT, MIN_Y};
use std::collections::HashMap;

pub type ChunkMap = HashMap<(i32, i32), Chunk>;

pub trait BlockAccess {
    /// None when the position is outside loaded chunks or the height limits.
    fn block(&self, x: i32, y: i32, z: i32) -> Option<&BlockState>;
    /// Returns false if the position is not loaded.
    fn set_block(&mut self, x: i32, y: i32, z: i32, b: &BlockState) -> bool;
}

impl BlockAccess for ChunkMap {
    fn block(&self, x: i32, y: i32, z: i32) -> Option<&BlockState> {
        if !(MIN_Y..MIN_Y + HEIGHT as i32).contains(&y) { return None; }
        self.get(&(x >> 4, z >> 4)).map(|c| c.get((x & 15) as usize, y, (z & 15) as usize))
    }
    fn set_block(&mut self, x: i32, y: i32, z: i32, b: &BlockState) -> bool {
        if !(MIN_Y..MIN_Y + HEIGHT as i32).contains(&y) { return false; }
        match self.get_mut(&(x >> 4, z >> 4)) {
            Some(c) => { c.set((x & 15) as usize, y, (z & 15) as usize, b); true }
            None => false,
        }
    }
}
