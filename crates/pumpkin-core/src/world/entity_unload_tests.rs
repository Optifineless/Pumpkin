use super::*;
use crate::entity::r#type::from_type;
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;

#[tokio::test]
async fn registry_cas_loss_never_marks_or_serializes_members() {
    let fixture = super::super::spawn_test_support::Fixture::new();
    let world = &fixture.world;
    let pos = Vector2::new(0, 0);
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&storage, None);
    let root = from_type(
        &EntityType::COW,
        Vector3::new(8.5, 64.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    let rider = from_type(
        &EntityType::COW,
        Vector3::new(8.5, 65.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(root.clone());
    world.add_entity_silent(rider.clone());
    root.get_entity().add_passenger(root.clone(), rider.clone());
    world.level.request_entity_unloads([pos]);
    let request = world
        .level
        .pending_entity_unloads()
        .into_iter()
        .find(|r| r.position == pos)
        .unwrap();
    let current = world.entities.load_full();
    // Real registry admission wins against the captured retirement incarnation.
    let newcomer = from_type(
        &EntityType::COW,
        Vector3::new(40.5, 64.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(newcomer);
    assert!(
        world
            .commit_entity_retirement(
                &request,
                &current,
                Some(Arc::new(Vec::new())),
                &[root.get_entity(), rider.get_entity()]
            )
            .is_none()
    );
    for member in [&root, &rider] {
        assert!(!member.get_entity().is_removed());
        assert!(member.get_entity().removal_reason.load().is_none());
        assert!(!member.get_entity().removed.load(Ordering::Acquire));
        assert!(
            world
                .get_entity_by_uuid(member.get_entity().entity_uuid)
                .is_some()
        );
    }
    assert!(storage.live.load(Ordering::Acquire));
    assert!(storage.data.lock().unwrap().is_empty());
    assert!(
        world
            .level
            .pending_entity_unloads()
            .iter()
            .any(|r| r.position == pos)
    );
    world.process_entity_chunk_lifecycle();
    assert!(root.get_entity().is_removed());
    assert!(rider.get_entity().is_removed());
    assert_eq!(
        storage.data.lock().unwrap()[0]
            .get_list("Passengers")
            .unwrap()
            .len(),
        1
    );
    fixture.finish().await;
}
