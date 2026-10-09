use super::{Entity, EntityBase, passive::cow::CowEntity};
use pumpkin_data::{entity::EntityPose, entity::EntityType};
use pumpkin_util::math::vector3::Vector3;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sleeping_entity_push_leaves_velocity_unchanged() {
    let fixture = super::death_test_world::DeathTestWorld::new().await;
    let world = fixture.world();
    let sleeper = CowEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::COW,
    ));
    let neighbor = CowEntity::new(Entity::new(
        world,
        Vector3::new(0.5, 64.0, 0.0),
        &EntityType::COW,
    ));
    let initial = Vector3::new(0.2, 0.1, 0.3);
    sleeper.get_entity().velocity.store(initial);
    sleeper.get_entity().pose.store(EntityPose::Sleeping);
    sleeper.push(neighbor.as_ref());
    assert_eq!(sleeper.get_entity().velocity.load(), initial);
    assert_eq!(
        neighbor.get_entity().velocity.load(),
        Vector3::new(0.0, 0.0, 0.0)
    );
    sleeper.get_entity().pose.store(EntityPose::Standing);
    sleeper.push(neighbor.as_ref());
    assert_ne!(sleeper.get_entity().velocity.load(), initial);
    assert_ne!(
        neighbor.get_entity().velocity.load(),
        Vector3::new(0.0, 0.0, 0.0)
    );
    fixture.server.shutdown().await;
}
