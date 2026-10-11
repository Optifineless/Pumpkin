use super::*;
use crate::entity::death_test_world::DeathTestWorld;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::{Hand, math::vector3::Vector3};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_integration_shield_credit_obeys_cooldown_acceptance() {
    let fixture = DeathTestWorld::new().await;
    let attacker = fixture.player("Attacker");
    let rejected_attacker = fixture.player("Rejected");
    let victim = fixture.player("Blocking");
    let living = &victim.living_entity;
    attacker
        .get_entity()
        .pos
        .store(Vector3::new(0.0, 100.0, 1.0));
    rejected_attacker
        .get_entity()
        .pos
        .store(attacker.position());
    let shield = ItemStack::new(1, &Item::SHIELD);
    living
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::OFF_HAND, shield.clone());
    living.set_active_hand(Hand::Left, shield.clone(), shield.get_max_use_time() - 5);
    assert!(living.is_blocking());
    let hit = |source: &dyn EntityBase| {
        victim.damage_with_context(
            &*victim,
            4.0,
            DamageType::PLAYER_ATTACK,
            None,
            Some(source),
            Some(source),
        )
    };
    let blocked_stat = || {
        victim.stats.lock().unwrap().get(
            StatisticCategory::Custom,
            CustomStatistic::DamageBlockedByShield as i32,
        )
    };
    let initial_health = living.health.load();
    assert!(!hit(&*attacker));
    assert_eq!(living.health.load(), initial_health);
    assert_eq!(living.hurt_cooldown.load(Relaxed), 20);
    assert_eq!(living.last_damage_taken.load(), 0.0);
    assert_eq!(
        living.get_kill_credit().unwrap().get_entity().entity_uuid,
        attacker.gameprofile.id
    );
    assert_eq!(blocked_stat(), 40);
    assert!(living.last_damage_type.lock().unwrap().is_none());
    living.hurt_by.lock().unwrap().player_memory_time = 70;
    assert!(!hit(&*rejected_attacker));
    assert_eq!(living.hurt_by.lock().unwrap().player_memory_time, 70);
    assert_eq!(blocked_stat(), 40);
    assert_eq!(
        living.get_kill_credit().unwrap().get_entity().entity_uuid,
        attacker.gameprofile.id
    );
    living.hurt_cooldown.store(10, Relaxed);
    assert!(!hit(&*rejected_attacker));
    assert_eq!(living.hurt_by.lock().unwrap().player_memory_time, 100);
    assert_eq!(blocked_stat(), 80);
    assert_eq!(
        living.get_kill_credit().unwrap().get_entity().entity_uuid,
        rejected_attacker.gameprofile.id
    );
    assert!(
        living
            .combat_tracker
            .lock()
            .unwrap()
            .get_killer_entry()
            .is_none()
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_integration_armored_excess_wears_once_and_drops_the_worn_stack() {
    let fixture = DeathTestWorld::new().await;
    let attacker = fixture.player("Attacker");
    let victim = fixture.player("Armored");
    let living = &victim.living_entity;
    let mut chestplate = ItemStack::new(1, &Item::IRON_CHESTPLATE);
    chestplate.set_damage(17);
    living
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::CHEST, chestplate);
    living.apply_current_equipment_attribute_modifiers();
    living.set_attribute_base(&Attributes::ARMOR, 14.0);
    living.set_attribute_base(&Attributes::ARMOR_TOUGHNESS, 0.0);
    let hit = |amount| {
        victim.damage_with_context(
            &*victim,
            amount,
            DamageType::PLAYER_ATTACK,
            None,
            Some(&*attacker),
            Some(&*attacker),
        )
    };
    let wear = || {
        living
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::CHEST)
            .get_damage()
    };
    // CombatRules: raw 6 against armor 20 is 1.92; the excess 4 of a raw 10 hit is 1.12.
    assert!(hit(6.0));
    assert!((living.health.load() - 18.08).abs() < 0.000_01);
    assert_eq!(wear(), 18);
    assert!(!hit(6.0));
    assert_eq!(wear(), 18);
    assert!(hit(10.0));
    assert!((living.health.load() - 16.96).abs() < 0.000_01);
    assert_eq!(wear(), 19);
    let entry = living
        .combat_tracker
        .lock()
        .unwrap()
        .get_killer_entry()
        .cloned()
        .unwrap();
    assert!((entry.damage - 1.92).abs() < 0.000_01);
    assert_eq!(living.last_damage_taken.load(), 10.0);
    assert!(victim.damage(&*victim, f32::MAX, DamageType::GENERIC_KILL));
    let dropped = fixture.world().entities.load_full();
    let dropped_armor: Vec<_> = dropped
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .filter_map(|item| {
            let stack = item.get_item_stack().lock().unwrap();
            (stack.item.id == Item::IRON_CHESTPLATE.id).then_some(stack.clone())
        })
        .collect();
    assert_eq!(dropped_armor.len(), 1);
    assert_eq!(dropped_armor[0].get_damage(), 19);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn damage_pipeline_wears_the_actual_victims_armor_without_a_world_lookup() {
    let fixture = DeathTestWorld::new().await;
    let victim = fixture.player("Armored");
    let living = &victim.living_entity;
    living.entity_equipment.lock().unwrap().put(
        &EquipmentSlot::CHEST,
        ItemStack::new(1, &Item::IRON_CHESTPLATE),
    );
    // A supplied Player remains the armor victim even before world registration.
    fixture
        .world()
        .players
        .store(std::sync::Arc::new(Vec::new()));
    assert!(victim.damage(&*victim, 4.0, DamageType::MOB_ATTACK));
    assert_eq!(
        living
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::CHEST)
            .get_damage(),
        1
    );
    crate::server::fixture_lifecycle::finish().await;
}
