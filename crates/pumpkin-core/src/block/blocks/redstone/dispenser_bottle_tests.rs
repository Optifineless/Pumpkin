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
    let mut properties = DispenserLikeProperties::default(&Block::DISPENSER);
    properties.facing = Facing::East;
    world.set_block_state(
        &position,
        properties.to_state_id(&Block::DISPENSER),
        BlockFlags::FORCE_STATE,
    );
    let dispenser = Arc::new(DispenserBlockEntity::new(position));
    dispenser.set_stack(0, ItemStack::new(3, &Item::EXPERIENCE_BOTTLE));
    world.add_block_entity(dispenser.clone());
    DispenserBlock.on_scheduled_tick(OnScheduledTickArgs {
        world: &world,
        block: &Block::DISPENSER,
        position: &position,
    });
    assert_eq!(dispenser.get_stack(0).item_count, 2);
    let entities = world.entities.load_full();
    assert_eq!(entities.len(), 1);
    let bottle = entities[0]
        .cast_any()
        .downcast_ref::<ExperienceBottleEntity>()
        .unwrap();
    assert_eq!(bottle.get_entity().pos.load(), Vector3::new(9.2, 64.5, 8.5));
    assert!(bottle.projectile_state().unwrap().owner_uuid().is_none());
    fixture.server.shutdown().await;
}
