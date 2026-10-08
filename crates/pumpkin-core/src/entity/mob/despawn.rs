//! Mob distance despawning and the inherited `LivingEntity` inactivity counter.

use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

use pumpkin_data::entity::{EntityType, MobCategory};
use pumpkin_util::Difficulty;

use super::{Mob, MobEntity};
use crate::world::brightness::DAYLIGHT_BRIGHTNESS;

// Mob.checkDespawn (Mob.java:701) hardcodes this inactivity gate and random bound.
const RANDOM_DESPAWN_MIN_NO_ACTION_TIME: i32 = 600;
const RANDOM_DESPAWN_CHANCE: u16 = 800;

impl MobEntity {
    pub fn check_despawn(&self, mob: &dyn Mob) {
        self.check_despawn_with_no_action_time(mob, Some(&self.living_entity.no_action_time));
    }

    /// Applies Mob.checkDespawn using the counter incremented by Mob.serverAiStep.
    pub fn check_despawn_with_no_action_time(
        &self,
        mob: &dyn Mob,
        no_action_time: Option<&AtomicI32>,
    ) {
        let entity = &self.living_entity.entity;
        // EnderDragon.checkDespawn is empty, including on Peaceful.
        if entity.entity_type == &EntityType::ENDER_DRAGON {
            return;
        }
        let world = entity.world.load();
        // Peaceful removal precedes both persistence checks in Mob.checkDespawn.
        if world.level_info.load().difficulty == Difficulty::Peaceful
            && !crate::entity::r#type::is_allowed_in_peaceful(entity.entity_type)
        {
            entity.remove();
            return;
        }
        // WitherBoss.checkDespawn resets inactivity instead of applying distance removal.
        if entity.entity_type == &EntityType::WITHER {
            if let Some(counter) = no_action_time {
                counter.store(0, Relaxed);
            }
            return;
        }
        let persistent =
            self.persistence_required.load(Relaxed) || mob.requires_custom_persistence();
        let pos = entity.pos.load();
        let nearest = world
            .players
            .load()
            .iter()
            .filter(|p| !p.is_spectator())
            .map(|p| p.position().squared_distance_to_vec(&pos))
            .reduce(f64::min);
        let Some(distance) = nearest else {
            return;
        };
        let category = entity.entity_type.category;
        let idle = no_action_time.map_or(0, |counter| counter.load(Relaxed));
        let roll = if !persistent && idle > RANDOM_DESPAWN_MIN_NO_ACTION_TIME {
            rand::random_range(0..RANDOM_DESPAWN_CHANCE)
        } else {
            1
        };
        let outcome = despawn_outcome(
            category,
            distance,
            persistent,
            mob.remove_when_far_away(distance),
            idle,
            roll,
        );
        if outcome.remove {
            entity.remove();
        }
        if outcome.reset_no_action_time
            && let Some(counter) = no_action_time
        {
            counter.store(0, Relaxed);
        }
    }
}

// AbstractFish/Axolotl.requiresCustomPersistence and removeWhenFarAway.
pub(super) fn is_bucket_mob(ty: &EntityType) -> bool {
    matches!(
        ty.resource_name,
        "cod" | "salmon" | "pufferfish" | "tropical_fish" | "tadpole" | "axolotl" | "sulfur_cube"
    )
}

pub(super) fn requires_custom_persistence<M: Mob + ?Sized>(mob: &M) -> bool {
    // Mob.requiresCustomPersistence tests being a passenger, not having passengers.
    mob.get_entity().has_vehicle()
        || mob.get_entity().is_leashed()
        || (is_bucket_mob(mob.get_entity().entity_type) && mob.spawned_from_bucket())
        // SulfurCube.requiresCustomPersistence also preserves an equipped body item.
        || (mob.get_entity().entity_type == &EntityType::SULFUR_CUBE
            && !mob
                .get_mob_entity()
                .living_entity
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&pumpkin_data::data_component_impl::EquipmentSlot::BODY)
                .is_empty())
        || mob
            .as_raider()
            .is_some_and(super::raider::Raider::has_active_raid)
}

pub(super) fn remove_when_far_away<M: Mob + ?Sized>(mob: &M, distance_sq: f64) -> bool {
    let entity = mob.get_entity();
    if let Some(raider) = mob.as_raider()
        && raider.has_active_raid()
    {
        return false;
    }
    if let Some(patrol) = mob.as_patrolling_monster() {
        // PatrollingMonster.removeWhenFarAway uses patrolling, not patrolLeader (Java line 99).
        return patrol_distance_removal(patrol.get_patrol_data(), distance_sq);
    }
    species_distance_removal(
        entity.entity_type,
        mob.spawned_from_bucket(),
        entity.custom_name.load().is_some(),
    )
}

fn patrol_distance_removal(patrol: &super::patrol::PatrolData, distance_sq: f64) -> bool {
    !patrol.patrolling.load(Relaxed) || distance_sq > 16384.0
}

fn species_distance_removal(ty: &EntityType, from_bucket: bool, named: bool) -> bool {
    match ty.resource_name {
        // Villager/WanderingTrader, AbstractGolem, Allay and Warden.removeWhenFarAway.
        "villager" | "wandering_trader" | "iron_golem" | "snow_golem" | "copper_golem"
        | "shulker" | "allay" | "warden" => false,
        "cod" | "salmon" | "pufferfish" | "tropical_fish" | "axolotl" => !from_bucket && !named,
        // AbstractNautilus, ZombieHorse, CamelHusk and Hoglin override Animal's false.
        // WaterAnimal and AgeableWaterCreature (Dolphin/Squid) inherit Mob's true.
        "nautilus" | "zombie_nautilus" | "zombie_horse" | "camel_husk" | "hoglin" | "dolphin"
        | "squid" | "glow_squid" => true,
        _ => !inherits_animal(ty),
    }
}

// Vanilla Animal's subclasses, including Parrot which has no Pumpkin as_animal hook yet.
pub(super) fn inherits_animal(ty: &EntityType) -> bool {
    matches!(
        ty.resource_name,
        "armadillo"
            | "axolotl"
            | "bee"
            | "camel"
            | "camel_husk"
            | "cat"
            | "chicken"
            | "cow"
            | "donkey"
            | "fox"
            | "frog"
            | "goat"
            | "happy_ghast"
            | "hoglin"
            | "horse"
            | "llama"
            | "mooshroom"
            | "mule"
            | "nautilus"
            | "ocelot"
            | "panda"
            | "parrot"
            | "pig"
            | "polar_bear"
            | "rabbit"
            | "sheep"
            | "skeleton_horse"
            | "sniffer"
            | "strider"
            | "trader_llama"
            | "turtle"
            | "wolf"
            | "zombie_horse"
            | "zombie_nautilus"
    )
}

// Cat/Ocelot.removeWhenFarAway hardcode the 2400-tick grace period.
pub(crate) const fn aged_wild_mob_can_despawn(protected: bool, tick_count: i32) -> bool {
    !protected && tick_count > 2400
}

pub struct DespawnOutcome {
    pub remove: bool,
    pub reset_no_action_time: bool,
}

/// Distance and inactivity part of Mob.checkDespawn, after finding an eligible player.
#[must_use]
pub fn despawn_outcome(
    category: &pumpkin_data::entity::MobCategory,
    distance_sq: f64,
    persistent: bool,
    remove_when_far_away: bool,
    no_action_time: i32,
    roll: u16,
) -> DespawnOutcome {
    let instant = f64::from(category.despawn_distance).powi(2);
    let near = f64::from(pumpkin_data::entity::MobCategory::NO_DESPAWN_DISTANCE).powi(2);
    DespawnOutcome {
        remove: !persistent
            && remove_when_far_away
            && (distance_sq > instant
                || (no_action_time > RANDOM_DESPAWN_MIN_NO_ACTION_TIME
                    && roll == 0
                    && distance_sq > near)),
        reset_no_action_time: distance_sq < near,
    }
}

// Monster inheritance differs from the hostile spawning category: these extend other bases.
pub(super) fn inherits_monster(entity_type: &EntityType) -> bool {
    entity_type.category == &MobCategory::MONSTER
        && !matches!(
            entity_type.resource_name,
            "camel_husk"
                | "ender_dragon"
                | "ghast"
                | "hoglin"
                | "magma_cube"
                | "phantom"
                | "shulker"
                | "slime"
                | "sulfur_cube"
                | "zombie_horse"
                | "zombie_nautilus"
        )
}

fn update_monster_no_action_time(counter: &AtomicI32, entity_type: &EntityType, brightness: f32) {
    // Raider.updateNoActionTime adds two even in darkness; Monster requires brightness > 0.5.
    let raider = matches!(
        entity_type.resource_name,
        "evoker" | "illusioner" | "pillager" | "ravager" | "vindicator" | "witch"
    );
    if raider || (inherits_monster(entity_type) && brightness > DAYLIGHT_BRIGHTNESS) {
        counter.fetch_add(2, Relaxed);
    }
}

// Monster.aiStep runs before LivingEntity.aiStep and its serverAiStep, including with NoAI.
pub(super) fn update_no_action_time<M: Mob + ?Sized>(mob: &M) {
    mob.get_mob_entity().ticks_lived.fetch_add(1, Relaxed);
    let living = &mob.get_mob_entity().living_entity;
    let entity = &living.entity;
    if let Some(raider) = mob.as_raider() {
        // Raider.aiStep resets active raiders pursuing players or iron golems before the +2.
        if living.health.load() > 0.0
            && raider.can_join_raid()
            && raider.has_active_raid()
            && mob.get_mob_entity().get_target().is_some_and(|target| {
                matches!(
                    target.get_entity().entity_type.resource_name,
                    "player" | "iron_golem"
                )
            })
        {
            living.no_action_time.store(0, Relaxed);
        }
    }
    if inherits_monster(entity.entity_type) {
        let world = entity.world.load();
        // Entity.getLightLevelDependentMagicValue returns zero outside loaded chunks.
        let brightness = if world.level.is_chunk_loaded(&entity.chunk_pos.load()) {
            world.get_light_level_dependent_magic_value(&entity.get_eye_pos().to_block_pos())
        } else {
            0.0
        };
        update_monster_no_action_time(&living.no_action_time, entity.entity_type, brightness);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_despawn_requires_inactivity_and_respects_persistence() {
        use pumpkin_data::entity::MobCategory;
        let check = |distance, persistent, far, idle, roll| {
            despawn_outcome(&MobCategory::MONSTER, distance, persistent, far, idle, roll)
        };
        assert!(!check(33.0f64.powi(2), false, true, 600, 0).remove);
        assert!(check(33.0f64.powi(2), false, true, 601, 0).remove);
        assert!(!check(33.0f64.powi(2), false, true, 601, 1).remove);
        assert!(check(129.0f64.powi(2), false, true, 0, 1).remove);
        assert!(!check(129.0f64.powi(2), true, true, 601, 0).remove);
        assert!(!check(129.0f64.powi(2), false, false, 601, 0).remove);
        assert!(check(31.0f64.powi(2), true, false, 601, 0).reset_no_action_time);
        assert!(
            despawn_outcome(
                &MobCategory::WATER_AMBIENT,
                65.0f64.powi(2),
                false,
                true,
                0,
                1
            )
            .remove
        );
    }

    #[test]
    fn light_inactivity_unlocks_distance_despawn_and_respects_inheritance() {
        let counter = AtomicI32::new(599);
        update_monster_no_action_time(&counter, &EntityType::ZOMBIE, 0.5);
        assert!(
            !despawn_outcome(
                &MobCategory::MONSTER,
                1089.0,
                false,
                true,
                counter.load(Relaxed),
                0
            )
            .remove
        );
        update_monster_no_action_time(&counter, &EntityType::ZOMBIE, 1.0);
        assert!(
            despawn_outcome(
                &MobCategory::MONSTER,
                1089.0,
                false,
                true,
                counter.load(Relaxed),
                0
            )
            .remove
        );
        for entity_type in [
            &EntityType::COW,
            &EntityType::HOGLIN,
            &EntityType::GHAST,
            &EntityType::SLIME,
            &EntityType::SULFUR_CUBE,
            &EntityType::CAMEL_HUSK,
            &EntityType::ZOMBIE_HORSE,
            &EntityType::ZOMBIE_NAUTILUS,
        ] {
            counter.store(599, Relaxed);
            update_monster_no_action_time(&counter, entity_type, 1.0);
            assert_eq!(counter.load(Relaxed), 599, "{}", entity_type.resource_name);
        }
        counter.store(599, Relaxed);
        update_monster_no_action_time(&counter, &EntityType::PILLAGER, 0.0);
        assert!(
            despawn_outcome(
                &MobCategory::MONSTER,
                1089.0,
                false,
                true,
                counter.load(Relaxed),
                0
            )
            .remove
        );
    }
}
