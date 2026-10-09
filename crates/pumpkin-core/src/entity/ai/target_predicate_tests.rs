use super::target_predicate::TargetPredicate;
use crate::{
    entity::{Entity, EntityBase, living::LivingEntity},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
};
use pumpkin_data::{
    data_component_impl::EquipmentSlot, entity::EntityType, item::Item, item_stack::ItemStack,
};
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering::Relaxed;

#[tokio::test]
async fn avoidance_followup_sneaking_target_range_and_scaling_opt_out() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let player = TestPlayer::new(&world);
    let tester = LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::SKELETON,
    ));
    let targeting = TargetPredicate::create_attackable().set_base_max_distance(10.0);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(9.0, 64.0, 0.0));
    assert!(targeting.test(&world, Some(&tester), player.player.as_ref()));
    player.player.get_entity().sneaking.store(true, Relaxed);
    assert!(!targeting.test(&world, Some(&tester), player.player.as_ref()));
    assert!(targeting.copy().ignore_distance_scaling_factor().test(
        &world,
        Some(&tester),
        player.player.as_ref()
    ));
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(7.0, 64.0, 0.0));
    assert!(targeting.test(&world, Some(&tester), player.player.as_ref()));
    player.player.get_entity().invisible.store(true, Relaxed);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(2.0, 64.0, 0.0));
    assert!(targeting.test(&world, Some(&tester), player.player.as_ref()));
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(2.01, 64.0, 0.0));
    assert!(!targeting.test(&world, Some(&tester), player.player.as_ref()));
}

#[tokio::test]
async fn avoidance_followup_visibility_uses_armor_and_matching_head_components() {
    let directory = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(directory.path());
    let target = LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::PLAYER,
    ));
    let skeleton = Entity::new(
        world.clone(),
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::SKELETON,
    );
    let zombie = Entity::new(world, Vector3::new(0.0, 64.0, 0.0), &EntityType::ZOMBIE);
    target.entity.invisible.store(true, Relaxed);
    assert!((target.visibility_percent(None) - 0.07).abs() < 1e-8);
    target.entity_equipment.lock().unwrap().put(
        &EquipmentSlot::CHEST,
        ItemStack::new(1, &Item::IRON_CHESTPLATE),
    );
    assert!((target.visibility_percent(None) - 0.175).abs() < 1e-8);
    target.entity.invisible.store(false, Relaxed);
    target.entity_equipment.lock().unwrap().put(
        &EquipmentSlot::HEAD,
        ItemStack::new(1, &Item::SKELETON_SKULL),
    );
    assert_eq!(target.visibility_percent(Some(&skeleton)), 0.5);
    assert_eq!(target.visibility_percent(Some(&zombie)), 1.0);
    target
        .entity_equipment
        .lock()
        .unwrap()
        .equipment
        .remove(&EquipmentSlot::HEAD);
    target.entity_equipment.lock().unwrap().put(
        &EquipmentSlot::MAIN_HAND,
        ItemStack::new(1, &Item::SKELETON_SKULL),
    );
    assert_eq!(target.visibility_percent(Some(&skeleton)), 1.0);
}

#[tokio::test]
async fn avoidance_followup_network_nan_visibility_stays_in_range() {
    use pumpkin_data::{data_component::DataComponent, data_component_impl::MobVisibilityImpl};
    use pumpkin_protocol::{
        codec::{data_component::deserialize, var_int::VarInt},
        ser::NetworkWriteExt,
    };
    use std::io::Cursor;

    let directory = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(directory.path());
    let target = LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(100.0, 64.0, 0.0),
        &EntityType::PLAYER,
    ));
    let tester = LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::SKELETON,
    ));
    // MobVisibility.STREAM_CODEC accepts FLOAT without the persistent codec's range validation.
    let mut bytes = Vec::new();
    bytes.write_var_int(&VarInt(2)).unwrap();
    bytes
        .write_var_int(&VarInt(i32::from(EntityType::SKELETON.id)))
        .unwrap();
    bytes.extend_from_slice(&[0x7f, 0xc0, 0x00, 0x00]);
    let component = deserialize(DataComponent::MobVisibility, &mut Cursor::new(bytes)).unwrap();
    let visibility = component
        .as_any()
        .downcast_ref::<MobVisibilityImpl>()
        .unwrap()
        .clone();
    assert!(visibility.visibility.is_nan());
    let mut head = ItemStack::new(1, &Item::SKELETON_SKULL);
    head.set_data_component(visibility);
    target
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::HEAD, head);
    assert!(
        target
            .visibility_percent(Some(tester.get_entity()))
            .is_nan()
    );
    let targeting = TargetPredicate::create_non_attackable().set_base_max_distance(10.0);
    assert!(targeting.test(&world, Some(&tester), &target));
    assert!(
        !targeting
            .ignore_distance_scaling_factor()
            .test(&world, Some(&tester), &target)
    );
}
