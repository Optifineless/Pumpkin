use super::*;

#[tokio::test]
async fn piercing_arrow_hits_two_targets_in_one_tick_and_advances_to_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    world.level.loaded_chunks.insert(
        pumpkin_util::math::vector2::Vector2::new(0, 0),
        pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
    );
    let first = Arc::new(Receiver {
        living: LivingEntity::new(Entity::new(
            world.clone(),
            Vector3::new(3.0, 65.0, 8.0),
            &EntityType::COW,
        )),
        hits: Mutex::default(),
        accepted: true,
    });
    let second = Arc::new(Receiver {
        living: LivingEntity::new(Entity::new(
            world.clone(),
            Vector3::new(5.0, 65.0, 8.0),
            &EntityType::COW,
        )),
        hits: Mutex::default(),
        accepted: true,
    });
    world
        .entities
        .store(Arc::new(vec![first.clone(), second.clone()]));
    let start = Vector3::new(1.0, 65.5, 8.0);
    let movement = Vector3::new(8.0, 0.0, 0.0);
    let arrow = arrow::ArrowEntity::new(Entity::new(world, start, &EntityType::ARROW), None);
    arrow.entity.velocity.store(movement);
    arrow.set_pierce_level(2);
    arrow.step_move_and_hit(&arrow, start, start + movement, movement);
    for target in [first, second] {
        assert_eq!(
            *target.hits.lock().unwrap(),
            vec![Hit {
                amount: 16.0,
                kind: DamageType::ARROW.id,
                direct: Some(arrow.entity.entity_id),
                cause: Some(arrow.entity.entity_id),
                raw_position: None,
            }]
        );
    }
    assert_eq!(arrow.pierced_entities.read().unwrap().len(), 2);
    assert_eq!(arrow.entity.pos.load(), start + movement);
    assert!(!arrow.entity.is_removed());
    crate::server::fixture_lifecycle::finish().await;
}
