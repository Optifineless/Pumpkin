use super::*;
use crate::{
    entity::death_test_world::DeathTestWorld,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::biome::Biome;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn experience_bottle_offhand_and_dispenser_launch() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let position = BlockPos::new(8, 64, 8);
    let context = DispenseContext {
        world: &world,
        position: &position,
        facing: Facing::East,
    };
    let mut stack = ItemStack::new(3, &Item::EXPERIENCE_BOTTLE);
    DispenserBlock::dispense_experience_bottle(&context, &mut stack);
    assert_eq!(stack.item_count, 2);
    let entities = world.entities.load_full();
    assert_eq!(entities.len(), 1);
    let bottle = entities[0]
        .cast_any()
        .downcast_ref::<ExperienceBottleEntity>()
        .unwrap();
    assert_eq!(bottle.get_entity().pos.load(), Vector3::new(9.2, 64.5, 8.5));
    assert!(bottle.projectile_state().unwrap().owner_uuid().is_none());
    fixture.server.shutdown().await;
    crate::server::fixture_lifecycle::finish().await;
}
