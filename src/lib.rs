#[cfg(feature = "desktop")]
pub mod desktop;
pub mod mesh;
pub mod mods;
pub mod octree;
pub mod physics;
pub mod planet;
pub mod vanilla;
pub mod vanilla_mesh;

pub mod blockstate;

pub mod state_registry;

pub mod block_world;

pub mod sprite;
pub mod world_mesh;

#[cfg(feature = "desktop")]
pub mod block_desktop;
#[cfg(feature = "desktop")]
pub mod textured_renderer;

pub mod items;

pub mod crafting;

pub mod collision;
