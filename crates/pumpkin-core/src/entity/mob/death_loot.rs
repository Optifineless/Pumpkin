use super::{EntityType, LivingEntity, Mob, equipment};
use rand::RngExt;
use std::sync::atomic::Ordering::Relaxed;

pub(super) fn should_drop_loot<T: Mob + ?Sized>(mob: &T) -> bool {
    let entity = mob.get_entity();
    mob_death_loot_allowed(
        entity.entity_type,
        entity.age.load(Relaxed) < 0,
        entity.world.load().level_info.load().game_rules.mob_drops,
    )
}

pub(super) fn should_drop_experience<T: Mob + ?Sized>(mob: &T) -> bool {
    let entity = mob.get_entity();
    mob_death_experience_allowed(entity.entity_type, entity.age.load(Relaxed) < 0)
}

/// Returns the inherited Mob/Animal XP reward for species with conditional overrides.
pub fn get_base_experience_reward<T: Mob + ?Sized>(mob: &T) -> u32 {
    // Animal, WaterAnimal and AgeableWaterCreature.getBaseExperienceReward
    let entity_type = mob.get_entity().entity_type;
    if mob.as_animal().is_some()
        || entity_type.category == &pumpkin_data::entity::MobCategory::WATER_CREATURE
        || entity_type.category == &pumpkin_data::entity::MobCategory::WATER_AMBIENT
        || entity_type.category == &pumpkin_data::entity::MobCategory::UNDERGROUND_WATER_CREATURE
    {
        return rand::rng().random_range(1..=3);
    }
    equipped_mob_experience(
        &mob.get_mob_entity().living_entity,
        entity_type.experience_reward,
    )
}

fn mob_death_experience_allowed(entity_type: &EntityType, baby: bool) -> bool {
    entity_type != &EntityType::TADPOLE
        && (!baby || has_monster_loot_override(entity_type) || entity_type == &EntityType::HOGLIN)
}

fn mob_death_loot_allowed(entity_type: &EntityType, baby: bool, mob_drops: bool) -> bool {
    mob_drops && (!baby || has_monster_loot_override(entity_type))
}

fn has_monster_loot_override(entity_type: &EntityType) -> bool {
    // Monster.shouldDropLoot / shouldDropExperience are inherited only by its subclasses.
    // EntityTypes supplies the concrete classes; AbstractCubeMob, Ghast and Phantom extend other bases.
    [
        &EntityType::BLAZE,
        &EntityType::BOGGED,
        &EntityType::BREEZE,
        &EntityType::CAVE_SPIDER,
        &EntityType::CREAKING,
        &EntityType::CREEPER,
        &EntityType::DROWNED,
        &EntityType::ELDER_GUARDIAN,
        &EntityType::ENDERMAN,
        &EntityType::ENDERMITE,
        &EntityType::EVOKER,
        &EntityType::GIANT,
        &EntityType::GUARDIAN,
        &EntityType::HUSK,
        &EntityType::ILLUSIONER,
        &EntityType::PARCHED,
        &EntityType::PIGLIN,
        &EntityType::PIGLIN_BRUTE,
        &EntityType::PILLAGER,
        &EntityType::RAVAGER,
        &EntityType::SILVERFISH,
        &EntityType::SKELETON,
        &EntityType::SPIDER,
        &EntityType::STRAY,
        &EntityType::VEX,
        &EntityType::VINDICATOR,
        &EntityType::WARDEN,
        &EntityType::WITCH,
        &EntityType::WITHER,
        &EntityType::WITHER_SKELETON,
        &EntityType::ZOGLIN,
        &EntityType::ZOMBIE,
        &EntityType::ZOMBIE_VILLAGER,
        &EntityType::ZOMBIFIED_PIGLIN,
    ]
    .contains(&entity_type)
}

/// Adds 1-3 XP for each occupied nonsaddle slot whose drop chance is at most one.
pub fn equipped_mob_experience(living: &LivingEntity, base: u32) -> u32 {
    // Mob.getBaseExperienceReward / EquipmentSlot.canIncreaseExperience
    if base == 0 {
        return 0;
    }
    // Shared lock order with spawn equipment: equipment before drop chances.
    let equipment = living
        .entity_equipment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let chances = living
        .equipment_drop_chances
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    equipment_experience_reward(base, &equipment, &chances, &mut rand::rng())
}

fn equipment_experience_reward(
    base: u32,
    equipment: &pumpkin_inventory::entity_equipment::EntityEquipment,
    chances: &rustc_hash::FxHashMap<pumpkin_data::data_component_impl::EquipmentSlot, f32>,
    rng: &mut impl rand::Rng,
) -> u32 {
    if base == 0 {
        return 0;
    }
    let mut reward = base;
    for slot in crate::entity::death_loot::equipment_slots_in_vanilla_order() {
        if !matches!(
            slot,
            pumpkin_data::data_component_impl::EquipmentSlot::Saddle(_)
        ) && !equipment.get(&slot).is_empty()
            && chances
                .get(&slot)
                .copied()
                .unwrap_or(equipment::DEFAULT_EQUIPMENT_DROP_CHANCE)
                <= 1.0
        {
            reward += rng.random_range(1..=3);
        }
    }
    reward
}

#[cfg(test)]
mod death_loot_tests {
    use super::*;
    use pumpkin_data::{data_component_impl::EquipmentSlot, item::Item, item_stack::ItemStack};
    use pumpkin_inventory::entity_equipment::EntityEquipment;
    use rand::SeedableRng;
    use rustc_hash::FxHashMap;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[expect(
        clippy::unwrap_used,
        reason = "Regression test requires real mob instances"
    )]
    async fn death_baby_loot_and_experience_use_the_class_overrides() {
        let fixture = crate::entity::death_test_world::DeathTestWorld::new().await;
        for (kind, loot, xp) in [
            (&EntityType::COW, false, false),
            (&EntityType::ZOMBIE, true, true),
            (&EntityType::HOGLIN, false, true),
            (&EntityType::SLIME, false, false),
            (&EntityType::MAGMA_CUBE, false, false),
            (&EntityType::TADPOLE, false, false),
        ] {
            let entity = fixture.mob(kind);
            entity.get_entity().age.store(-24000, Relaxed);
            let mob = entity.get_mob().unwrap();
            assert_eq!(mob.should_drop_loot(), loot, "{} loot", kind.resource_name);
            assert_eq!(
                mob.should_drop_experience(),
                xp,
                "{} XP",
                kind.resource_name
            );
            entity.get_entity().age.store(0, Relaxed);
            assert!(mob.should_drop_loot());
            assert_eq!(mob.should_drop_experience(), kind != &EntityType::TADPOLE);
        }
    }

    #[test]
    fn death_mob_experience_counts_hands_and_armor_but_skips_preserved_items_and_saddles() {
        let mut equipment = EntityEquipment::new();
        equipment.put(
            &EquipmentSlot::MAIN_HAND,
            ItemStack::new(1, &Item::IRON_SWORD),
        );
        equipment.put(&EquipmentSlot::HEAD, ItemStack::new(1, &Item::IRON_HELMET));
        equipment.put(
            &EquipmentSlot::CHEST,
            ItemStack::new(1, &Item::IRON_CHESTPLATE),
        );
        equipment.put(&EquipmentSlot::SADDLE, ItemStack::new(1, &Item::SADDLE));
        let mut chances = FxHashMap::default();
        chances.insert(EquipmentSlot::CHEST, 2.0);
        let mut rng = rand::rngs::StdRng::seed_from_u64(4);
        let reward = equipment_experience_reward(5, &equipment, &chances, &mut rng);
        assert_eq!(reward, 10);
        chances.insert(EquipmentSlot::MAIN_HAND, 2.0);
        chances.insert(EquipmentSlot::HEAD, 2.0);
        assert_eq!(
            equipment_experience_reward(5, &equipment, &chances, &mut rng),
            5
        );
        assert_eq!(
            equipment_experience_reward(0, &equipment, &FxHashMap::default(), &mut rng),
            0
        );
    }
}
