use super::super::spawn_test_support::Fixture;
use crate::entity::{EntityBase, spawn_mount, r#type::from_type};
use pumpkin_data::entity::{EntityType, MobCategory};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use std::sync::Arc;

fn entity(fixture: &Fixture, ty: &'static EntityType) -> Arc<dyn EntityBase> {
    from_type(
        ty,
        Vector3::new(8.5, 64.0, 8.5),
        &fixture.world,
        uuid::Uuid::new_v4(),
    )
}

#[tokio::test]
async fn natural_and_non_save_admission_consume_finalized_mounts_once() {
    let fixture = Fixture::new();
    let world = &fixture.world;
    let drowned = entity(&fixture, &EntityType::DROWNED);
    let mount = entity(&fixture, &EntityType::ZOMBIE_NAUTILUS);
    spawn_mount::queue_spawn_mount(&drowned, mount.clone());
    assert!(world.insert_spawned_entity(&drowned, false));
    assert_eq!(world.entities.load().len(), 2);
    assert!(Arc::ptr_eq(
        &drowned.get_entity().get_vehicle().unwrap(),
        &mount
    ));
    let state = world.spawn_state.load();
    assert_eq!(
        state.mob_category_counts.0[MobCategory::MONSTER.id]
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    state.after_spawn(
        &EntityType::DROWNED,
        &drowned.get_entity().block_pos.load(),
        world,
    );
    assert_eq!(
        state.mob_category_counts.0[MobCategory::MONSTER.id]
            .load(std::sync::atomic::Ordering::Relaxed),
        2
    );
    let zombie = entity(&fixture, &EntityType::ZOMBIE);
    let chicken = entity(&fixture, &EntityType::CHICKEN);
    spawn_mount::queue_spawn_mount(&zombie, chicken.clone());
    world.spawn_entity_non_save(zombie.clone());
    assert_eq!(world.entities.load().len(), 4);
    assert!(Arc::ptr_eq(
        &zombie.get_entity().get_vehicle().unwrap(),
        &chicken
    ));
    assert_eq!(
        state.mob_category_counts.0[MobCategory::MONSTER.id]
            .load(std::sync::atomic::Ordering::Relaxed),
        2
    );
    fixture.finish().await;
}

#[tokio::test]
async fn tree_uuid_rejection_and_cancelled_admission_leave_live_chicken_untouched() {
    let fixture = Fixture::new();
    let world = &fixture.world;
    let chicken = entity(&fixture, &EntityType::CHICKEN);
    assert!(world.spawn_entity(chicken.clone()));
    let zombie = entity(&fixture, &EntityType::ZOMBIE);
    spawn_mount::queue_existing_chicken(&zombie, chicken.clone());
    assert!(!chicken.get_entity().has_passengers());
    assert!(!world.admit_spawn_tree(&zombie, true, || false));
    let mut nbt = NbtCompound::new();
    chicken.write_nbt(&mut nbt);
    assert_eq!(nbt.get_bool("IsChickenJockey"), Some(false));
    assert!(!chicken.get_entity().has_passengers());
    assert!(zombie.get_entity().get_vehicle().is_none());
    spawn_mount::queue_existing_chicken(&zombie, chicken.clone());
    assert!(world.spawn_entity(zombie.clone()));
    chicken.write_nbt(&mut nbt);
    assert_eq!(nbt.get_bool("IsChickenJockey"), Some(true));
    assert!(!spawn_mount::available_chicken(&chicken));

    let duplicate = from_type(
        &EntityType::CHICKEN,
        Vector3::new(8.5, 64.0, 8.5),
        world,
        chicken.get_entity().entity_uuid,
    );
    let rejected = entity(&fixture, &EntityType::ZOMBIE);
    spawn_mount::queue_spawn_mount(&rejected, duplicate.clone());
    assert!(!world.insert_spawned_entity(&rejected, false));
    assert_eq!(world.entities.load().len(), 2);
    assert!(rejected.get_entity().get_vehicle().is_none());
    assert!(!duplicate.get_entity().has_passengers());

    let riding_chicken = entity(&fixture, &EntityType::CHICKEN);
    spawn_mount::attach_unpublished(&zombie, riding_chicken.clone());
    assert!(!spawn_mount::available_chicken(&riding_chicken));
    let dead_chicken = entity(&fixture, &EntityType::CHICKEN);
    dead_chicken.get_living_entity().unwrap().health.store(0.0);
    assert!(!spawn_mount::available_chicken(&dead_chicken));
    fixture.finish().await;
}

#[tokio::test]
async fn ordinary_unload_and_restart_restore_nested_riders_and_equipment() {
    use pumpkin_data::{data_component_impl::EquipmentSlot, item::Item, item_stack::ItemStack};
    let fixture = Fixture::new();
    let world = &fixture.world;
    let root = entity(&fixture, &EntityType::ZOMBIE_NAUTILUS);
    let rider = entity(&fixture, &EntityType::DROWNED);
    let passenger = entity(&fixture, &EntityType::CHICKEN);
    rider
        .get_mob()
        .unwrap()
        .get_mob_entity()
        .set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, &Item::TRIDENT));
    rider
        .get_living_entity()
        .unwrap()
        .equipment_drop_chances
        .lock()
        .unwrap()
        .insert(EquipmentSlot::MAIN_HAND, 0.0);
    spawn_mount::attach_unpublished(&root, rider.clone());
    spawn_mount::attach_unpublished(&rider, passenger.clone());
    assert!(world.spawn_entity_with_passengers(&root));
    // A passenger over the boundary still belongs to the root's entity chunk on unload.
    passenger
        .get_entity()
        .set_pos(Vector3::new(16.1, 65.0, 8.5));
    let pos = Vector2::new(0, 0);
    let chunk = world.level.get_entity_chunk(pos).await;
    world.make_chunk_entities_live(&chunk, None);
    world.remove_entities_in_chunks([pos]).await;
    assert!(world.entities.load().is_empty());
    let saved = chunk.data.lock().unwrap().clone();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].get_list("Passengers").unwrap().len(), 1);
    let ids = [
        root.get_entity().entity_uuid,
        rider.get_entity().entity_uuid,
        passenger.get_entity().entity_uuid,
    ];
    world.level.clean_entity_chunks([pos]);
    let fixture = fixture.restart().await;
    let world = &fixture.world;
    let chunk = world.level.get_entity_chunk(pos).await;
    assert_eq!(chunk.data.lock().unwrap().len(), 1);
    world.make_chunk_entities_live(&chunk, None);
    assert_eq!(world.entities.load().len(), 3);
    let root = world.get_entity_by_uuid(ids[0]).unwrap();
    let rider = world.get_entity_by_uuid(ids[1]).unwrap();
    let passenger = world.get_entity_by_uuid(ids[2]).unwrap();
    assert!(Arc::ptr_eq(
        &rider.get_entity().get_vehicle().unwrap(),
        &root
    ));
    assert!(Arc::ptr_eq(
        &passenger.get_entity().get_vehicle().unwrap(),
        &rider
    ));
    let living = rider.get_living_entity().unwrap();
    assert_eq!(
        living
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::MAIN_HAND)
            .item,
        &Item::TRIDENT
    );
    assert_eq!(
        living
            .equipment_drop_chances
            .lock()
            .unwrap()
            .get(&EquipmentSlot::MAIN_HAND),
        Some(&0.0)
    );
    world.make_chunk_entities_live(&chunk, None);
    assert_eq!(world.entities.load().len(), 3);
    fixture.finish().await;
}

#[tokio::test]
async fn restored_tree_skips_unknown_passengers() {
    use pumpkin_nbt::tag::NbtTag;
    let fixture = Fixture::new();
    let root = entity(&fixture, &EntityType::CHICKEN);
    let passenger = entity(&fixture, &EntityType::ZOMBIE);
    let mut nbt = NbtCompound::new();
    root.write_nbt(&mut nbt);
    let mut valid = NbtCompound::new();
    passenger.write_nbt(&mut valid);
    let mut unknown = NbtCompound::new();
    unknown.put_string("id", "minecraft:unknown_entity".to_owned());
    nbt.put(
        "Passengers",
        NbtTag::List(vec![NbtTag::Compound(unknown), NbtTag::Compound(valid)]),
    );
    fixture.world.restore_entity_tree(&nbt, None);
    assert_eq!(fixture.world.entities.load().len(), 2);
    let restored = fixture
        .world
        .get_entity_by_uuid(passenger.get_entity().entity_uuid)
        .unwrap();
    assert_eq!(
        restored
            .get_entity()
            .get_vehicle()
            .unwrap()
            .get_entity()
            .entity_uuid,
        root.get_entity().entity_uuid
    );
    fixture.finish().await;
}
