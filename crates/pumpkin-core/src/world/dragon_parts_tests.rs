use super::*;
use crate::{entity::r#type::from_type, world::spawn_test_support::Fixture};
use pumpkin_data::{damage::DamageType, entity::EntityType};
use pumpkin_util::math::vector3::Vector3;

#[tokio::test]
async fn dragon_parts_follow_tracking() {
    let fixture = Fixture::new();
    let world = &fixture.world;
    let entity = from_type(
        &EntityType::ENDER_DRAGON,
        Vector3::default(),
        world,
        uuid::Uuid::new_v4(),
    );
    let dragon = entity
        .cast_any()
        .downcast_ref::<EnderDragonEntity>()
        .unwrap();
    let id = entity.get_entity().entity_id;
    assert!(world.get_entity_or_part(id + 1).is_none());
    assert!(world.spawn_entity(entity.clone()));
    for (index, part) in dragon.parts.iter().enumerate() {
        assert_eq!(part.entity.entity_id, id + index as i32 + 1);
        let found = world.get_entity_or_part(part.entity.entity_id).unwrap();
        assert_eq!(found.get_entity().entity_uuid, part.entity.entity_uuid);
        assert!(world.get_entity_by_id(part.entity.entity_id).is_none());
    }
    assert_eq!(world.entities.load().len(), 1);
    world.entity_tracker.remove_entity(entity.as_ref(), world);
    assert!(
        dragon
            .parts
            .iter()
            .all(|part| world.get_entity_or_part(part.entity.entity_id).is_none())
    );
    world.entity_tracker.add_entity(&entity, world);
    assert!(world.get_entity_or_part(id + 1).is_some());
    world.remove_entity(entity.as_ref());
    assert!(world.get_entity_or_part(id + 1).is_none());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dragon_parts_forward_damage_with_head_and_neck_exemptions() {
    let dir = tempfile::tempdir().unwrap();
    let server = crate::server::combat_test_support::server(dir.path());
    let world = crate::server::combat_test_support::world(&server, dir.path());
    for (index, expected) in [(0, 8.0), (1, 8.0), (2, 3.0)] {
        let entity = from_type(
            &EntityType::ENDER_DRAGON,
            Vector3::default(),
            &world,
            uuid::Uuid::new_v4(),
        );
        let dragon = entity
            .cast_any()
            .downcast_ref::<EnderDragonEntity>()
            .unwrap();
        assert!(world.spawn_entity(entity.clone()));
        let part = &dragon.parts[index];
        let before = dragon.mob_entity.living_entity.health.load();
        assert!(part.damage_with_context(
            part.as_ref(),
            8.0,
            DamageType::GENERIC,
            None,
            None,
            None
        ));
        assert_eq!(
            dragon.mob_entity.living_entity.health.load(),
            before - expected
        );
    }
    world.level.shutdown().await.unwrap();
}
