use super::*;
use pumpkin_data::item::Item;
#[test]
fn death_mob_equipment_needs_recent_player_credit_or_preservation_and_obeys_vanishing() {
    let mut sword = ItemStack::new(1, &Item::IRON_SWORD);
    assert!(!mob_equipment_drop_eligible(&sword, false, false));
    assert!(mob_equipment_drop_eligible(&sword, true, false));
    assert!(mob_equipment_drop_eligible(&sword, false, true));
    sword.add_enchantment(&Enchantment::VANISHING_CURSE, 1);
    assert!(!mob_equipment_drop_eligible(&sword, true, true));
}

#[test]
fn death_experience_keeps_player_and_mob_eligibility_separate() {
    // Environmental player death, mob_drops=false: Player isAlwaysExperienceDropper.
    assert!(experience_drop_eligible(true, false, false, false, false));
    assert!(!experience_drop_eligible(true, true, true, true, true));
    assert!(experience_drop_eligible(false, false, true, true, true));
    assert!(!experience_drop_eligible(false, false, false, true, true));
    assert!(!experience_drop_eligible(false, false, true, false, true));
    assert!(!experience_drop_eligible(false, false, true, true, false));
}

#[test]
fn death_equipment_looting_bonus_requires_a_player_killer() {
    let effect = &Enchantment::LOOTING.effects.equipment_drops[0];
    assert!(equipment_drop_requirement_matches(
        effect,
        &EntityType::PLAYER,
        Some(&EntityType::ARROW)
    ));
    assert!(!equipment_drop_requirement_matches(
        effect,
        &EntityType::ZOMBIE,
        Some(&EntityType::ZOMBIE)
    ));
}

#[test]
fn death_looting_uses_only_enchantments_active_in_the_equipped_slot() {
    let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    sword.add_enchantment(&Enchantment::LOOTING, 3);
    let mut levels = Vec::new();
    visit_equipment_enchantments(&EquipmentSlot::OFF_HAND, &sword, |_, level| {
        levels.push(level);
    });
    assert!(levels.is_empty());
    visit_equipment_enchantments(&EquipmentSlot::MAIN_HAND, &sword, |_, level| {
        levels.push(level);
    });
    assert_eq!(levels, [3]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_assisted_fall_uses_the_real_landing_path() {
    use crate::entity::death_test_world::DeathTestWorld;
    let fixture = DeathTestWorld::new().await;
    let attacker = fixture.player("Assistant");
    let victim = fixture.mob(&EntityType::COW);
    let living = victim.get_living_entity().unwrap();
    assert!(victim.damage_with_context(
        &*victim,
        1.0,
        DamageType::PLAYER_ATTACK,
        None,
        Some(&*attacker),
        Some(&*attacker)
    ));
    living.hurt_cooldown.store(0, Relaxed);
    living.fall(&*victim, -30.0, false, false);
    assert_eq!(living.fall_distance.load(), 30.0);
    living.fall(&*victim, 0.0, true, false);
    assert_eq!(living.health.load(), 0.0);
    assert_eq!(living.fall_distance.load(), 0.0);
    let message = LivingEntity::get_death_message(&*victim, DamageType::FALL, None, None);
    assert!(format!("{message:?}").contains("death.fell.assist"));
    assert!(format!("{message:?}").contains("Assistant"));
    let mut saved = pumpkin_nbt::compound::NbtCompound::new();
    living.write_living_nbt(&mut saved);
    assert_eq!(saved.get_float("FallDistance"), Some(0.0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_ordinary_message_does_not_use_expired_combat_credit() {
    use crate::entity::death_test_world::DeathTestWorld;
    let fixture = DeathTestWorld::new().await;
    let attacker = fixture.player("ExpiredAttacker");
    let victim = fixture.mob(&EntityType::COW);
    let living = victim.get_living_entity().unwrap();
    assert!(victim.damage_with_context(
        &*victim,
        1.0,
        DamageType::PLAYER_ATTACK,
        None,
        Some(&*attacker),
        Some(&*attacker)
    ));
    victim.get_mob().unwrap().get_mob_entity().set_no_ai(true);
    victim.get_entity().velocity.store(Vector3::default());
    for _ in 0..101 {
        living.tick(&*victim, &fixture.server);
    }
    assert!(living.get_kill_credit().is_none());
    assert!(living.combat_tracker.lock().unwrap().is_in_combat());
    assert!(victim.damage_with_context(
        &*victim,
        f32::MAX,
        DamageType::GENERIC_KILL,
        None,
        None,
        None
    ));
    let message = LivingEntity::get_death_message(&*victim, DamageType::GENERIC_KILL, None, None);
    assert!(!format!("{message:?}").contains("ExpiredAttacker"));
    assert!(!format!("{message:?}").contains(".player"));
}
