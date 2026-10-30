//! A working compiled-in mod. Run: cargo run --example flight_mod --no-default-features
use glam::{DVec3, IVec3};
use orbital_voxel::{
    mods::{Command, Event, NativeMod},
    physics::{example_ship, BodyHandle, Physics},
};
#[derive(Default)]
struct ConstantThrust {
    ship: Option<BodyHandle>,
}
impl NativeMod for ConstantThrust {
    fn on_event(&mut self, event: &Event, commands: &mut Vec<Command>) {
        match event {
            Event::StructureSpawned(body) => self.ship = Some(*body),
            Event::FixedTick { .. } => {
                if let Some(body) = self.ship {
                    commands.push(Command::Accelerate {
                        body,
                        acceleration: DVec3::Y * 5.0,
                    });
                }
            }
        }
    }
}
fn main() {
    let mut world = Physics::new(vec![]);
    world.mods.register(ConstantThrust::default());
    let ship = world
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
    println!(
        "Mod-driven ship after 1 second: position {:?}, velocity {:?}",
        world.pose(ship).0,
        world.velocity(ship)
    );
}
