use super::*;
use crate::entity::ai::goal::species_avoid_entity::tests::floor;
use crate::{entity::Entity, server::combat_test_support, world::World};
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering::Relaxed;

fn trader_threat(
    world: &Arc<World>,
    entity_type: &'static EntityType,
    x: f64,
) -> Arc<crate::entity::living::LivingEntity> {
    Arc::new(crate::entity::living::LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(x, 64.0, 8.5),
        entity_type,
    )))
}

#[tokio::test]
async fn avoidance_followup_trader_avoids_zombified_piglin() {
    use crate::entity::passive::wandering_trader::WanderingTraderEntity;
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    floor(&world);
    let trader = WanderingTraderEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::WANDERING_TRADER,
    ));
    trader.get_entity().on_ground.store(true, Relaxed);
    let piglin = trader_threat(&world, &EntityType::ZOMBIFIED_PIGLIN, 10.5);
    world.entities.store(Arc::new(vec![piglin]));
    let mut selector = trader.mob_entity.goals_selector.lock().unwrap();
    let mut goals: Vec<_> = selector.goals_for_test::<AvoidEntityGoal>().collect();
    assert!((0..256).any(|_| goals[0].can_start(trader.as_ref())));
    assert_eq!(goals.len(), 7);
}

#[tokio::test]
async fn avoidance_followup_trader_selects_nearest_zombie_subclass() {
    use crate::entity::passive::wandering_trader::WanderingTraderEntity;
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    floor(&world);
    let trader = WanderingTraderEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::WANDERING_TRADER,
    ));
    trader.get_entity().on_ground.store(true, Relaxed);
    let far = trader_threat(&world, &EntityType::ZOMBIE, 13.5);
    let near = trader_threat(&world, &EntityType::HUSK, 10.5);
    world.entities.store(Arc::new(vec![far, near.clone()]));
    let mut goal = AvoidEntityGoal::new(&EntityType::ZOMBIE, 8.0, 0.5, 0.5, None);
    assert!((0..256).any(|_| goal.can_start(trader.as_ref())));
    assert_eq!(
        goal.threat().unwrap().get_entity().entity_id,
        near.get_entity().entity_id
    );
}
