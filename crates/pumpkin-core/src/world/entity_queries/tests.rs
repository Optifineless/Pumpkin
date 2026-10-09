use super::*;
use crate::{
    entity::Entity, net::java::combat_test_support::TestPlayer, server::combat_test_support,
};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;

#[tokio::test]
async fn avoidance_followup_box_query_borrows_handles_and_matches_snapshot_query() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let inside: Arc<dyn EntityBase> = Arc::new(Entity::new(
        world.clone(),
        Vector3::new(1.0, 64.0, 1.0),
        &EntityType::ITEM,
    ));
    let outside: Arc<dyn EntityBase> = Arc::new(Entity::new(
        world.clone(),
        Vector3::new(50.0, 64.0, 50.0),
        &EntityType::ITEM,
    ));
    world
        .entities
        .store(Arc::new(vec![inside.clone(), outside]));
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(2.0, 64.0, 2.0));
    let aabb = BoundingBox::new(Vector3::new(0.0, 63.0, 0.0), Vector3::new(4.0, 67.0, 4.0));
    let count = Arc::strong_count(&inside);
    let mut ids = Vec::new();
    world.for_each_in_box(&aabb, |entity| {
        if entity.get_entity().entity_id == inside.get_entity().entity_id {
            assert_eq!(
                Arc::strong_count(&inside),
                count,
                "the callback must borrow the intersecting entity"
            );
        }
        ids.push(entity.get_entity().entity_id);
    });
    assert_eq!(
        ids,
        world
            .get_all_at_box(&aabb)
            .iter()
            .map(|entity| entity.get_entity().entity_id)
            .collect::<Vec<_>>()
    );
    assert_eq!(ids.len(), 2);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(20.0, 64.0, 20.0));
    let player_count = Arc::strong_count(&player.player);
    world.for_each_in_box(&aabb, |_| {
        assert_eq!(Arc::strong_count(&player.player), player_count);
    });
}
