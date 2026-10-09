use super::*;
use crate::entity::death_test_world::DeathTestWorld;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dead_mob_dispenser_is_not_sheared() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let sheep = fixture.mob(&EntityType::SHEEP);
    sheep.get_entity().set_pos(Vector3::new(9.5, 64.0, 8.5));
    let living = sheep.get_living_entity().unwrap();
    living.health.store(0.0);
    let position = BlockPos::new(8, 64, 8);
    let ctx = DispenseContext {
        world: &world,
        position: &position,
        facing: Facing::East,
    };
    let tool = ItemStack::new(1, &Item::SHEARS);
    assert!(!DispenserBlock::shear_entity_in_front(&ctx, &tool));
    assert!(
        sheep
            .get_mob()
            .unwrap()
            .as_shearable()
            .unwrap()
            .ready_for_shearing()
    );
    living.health.store(5.0);
    assert!(DispenserBlock::shear_entity_in_front(&ctx, &tool));
    assert!(
        !sheep
            .get_mob()
            .unwrap()
            .as_shearable()
            .unwrap()
            .ready_for_shearing()
    );
    fixture.server.shutdown().await;
}
