use super::avoid_entity::AvoidEntityGoal;
use crate::{
    entity::{
        Entity,
        passive::{llama::LlamaEntity, rabbit::RabbitEntity},
    },
    server::combat_test_support,
};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;

#[tokio::test]
async fn avoidance_followup_threat_search_borrows_candidates_before_selection() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let rabbit = RabbitEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::RABBIT,
    ));
    let llama = LlamaEntity::new(Entity::new(
        world.clone(),
        Vector3::new(10.5, 64.0, 8.5),
        &EntityType::LLAMA,
    ));
    world.entities.store(Arc::new(vec![llama.clone()]));
    let count = Arc::strong_count(&llama);
    assert!(
        AvoidEntityGoal::find_threat(rabbit.as_ref(), 8.0, |target| {
            assert_eq!(
                Arc::strong_count(&llama),
                count,
                "find_threat must inspect borrowed handles before selecting a threat"
            );
            target.get_entity().entity_type == &EntityType::LLAMA
        })
        .is_some()
    );
}
