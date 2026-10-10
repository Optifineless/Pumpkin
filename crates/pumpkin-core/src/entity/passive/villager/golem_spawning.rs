use std::sync::{Arc, atomic::Ordering::Relaxed};

use pumpkin_data::{
    attributes::Attributes,
    entity::{EntityPose, EntityType},
    environment_attribute::Activity,
};
use rand::RngExt;

use super::VillagerEntity;
use crate::entity::{
    EntityBase,
    ai::{
        brain::{
            Brain,
            memory::{PackedMemories, types},
        },
        target_predicate::TargetPredicate,
    },
    living::LivingEntity,
    mob::{Mob, spawn::SpawnReason},
    passive::iron_golem::IronGolemEntity,
    spawn_util::{SpawnStrategy, try_spawn_mob_with_random},
};

// GolemSensor.java:12-13, 44: 599 TTL expires on the following Brain.tick (600 entity ticks).
const GOLEM_SCAN_RATE: i32 = 200;
const MEMORY_TIME_TO_LIVE: i64 = 599;
// Villager.java:821-826, 846-852, 913; VillagerPanicTrigger.java:35-37.
const LAST_SLEPT_WINDOW: i64 = 24_000;
const VILLAGER_SEARCH_RANGE: f64 = 10.0;
const PANIC_CHECK_INTERVAL: i64 = 100;
const PANIC_VILLAGERS_NEEDED: usize = 3;
const GOSSIP_VILLAGERS_NEEDED: usize = 5;
const GOSSIP_INTERVAL: i64 = 1_200;
const GOSSIP_DISTANCE_SQ: f64 = 5.0;
// VillagerGoalPackages.getIdlePackage -> InteractWith.of.
const INTERACTION_RANGE: f64 = 8.0;
const SPAWN_ATTEMPTS: i32 = 10;
const SPAWN_RANGE_XZ: i32 = 8;
const SPAWN_RANGE_Y: i32 = 6;

#[cfg(test)]
mod tests;

pub(super) fn initial_scan_delay() -> i32 {
    // Sensor.randomlyDelayStart.
    rand::rng().random_range(0..GOLEM_SCAN_RATE)
}

pub(super) fn make_brain(packed: &PackedMemories) -> Brain {
    let mut brain = Brain::default();
    brain.register_memory(types::LAST_SLEPT.id());
    brain.register_memory(types::LAST_WOKEN.id());
    brain.register_memory(types::HOME.id());
    brain.register_memory(types::CANT_REACH_WALK_TARGET_SINCE.id());
    brain.register_memory(types::DOORS_TO_CLOSE.id());
    brain.register_memory(types::GOLEM_DETECTED_RECENTLY.id());
    brain.register_memory(types::INTERACTION_TARGET.id());
    brain.register_memory(types::MEETING_POINT.id());
    brain.load_packed(packed);
    brain
}

impl VillagerEntity {
    pub(super) fn record_last_slept(&self, timestamp: i64) {
        // SleepInBed.start records actual sleep, not ownership of a bed.
        self.mob_entity
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set(types::LAST_SLEPT, timestamp);
    }

    fn golem_spawn_conditions_met(game_time: i64, brain: &Brain) -> bool {
        // Villager.golemSpawnConditionsMet in 26.3 has no LAST_WORKED_AT_POI condition.
        brain
            .get(types::LAST_SLEPT)
            .is_some_and(|slept| game_time - slept < LAST_SLEPT_WINDOW)
    }

    fn wants_to_spawn_golem(&self, timestamp: i64) -> bool {
        let brain = self
            .mob_entity
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::golem_spawn_conditions_met(timestamp, &brain)
            && !brain.has_memory_value(types::GOLEM_DETECTED_RECENTLY.id())
    }

    fn golem_detected(&self) {
        // GolemSensor.golemDetected, also called on every nearby villager after a summon.
        self.mob_entity
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_with_expiry(types::GOLEM_DETECTED_RECENTLY, true, MEMORY_TIME_TO_LIVE);
    }

    pub(super) fn nearby_entities(&self) -> Vec<Arc<dyn EntityBase>> {
        let entity = self.get_entity();
        let range = self
            .mob_entity
            .living_entity
            .get_attribute_value(&Attributes::FOLLOW_RANGE);
        let bounds = entity.bounding_box.load().expand(range, range, range);
        entity.world.load().get_entities_at_box(&bounds)
    }

    fn check_for_nearby_golem(&self, nearby: &[Arc<dyn EntityBase>]) {
        // NearestLivingEntitySensor.doTick includes living golems without a visibility test.
        if nearby.iter().any(|other| {
            other.get_entity().entity_type == &EntityType::IRON_GOLEM
                && other
                    .get_living_entity()
                    .is_some_and(LivingEntity::is_alive)
        }) {
            self.golem_detected();
        }
    }

    pub(super) fn golem_ai_step(&self) {
        self.golem_ai_step_with_random(&mut rand::rng());
    }

    fn golem_ai_step_with_random(&self, random: &mut impl rand::Rng) {
        self.mob_entity.tick_brain(self);
        let scan_golems = self.golem_scan_ticks.fetch_sub(1, Relaxed) <= 1;
        let timestamp = self.get_entity().world.load().get_world_age();
        let panic_check = timestamp % PANIC_CHECK_INTERVAL == 0;
        let social_check =
            timestamp % i64::from(crate::entity::ai::brain::sensing::DEFAULT_SCAN_RATE) == 0;
        if !scan_golems && !panic_check && !social_check {
            return;
        }
        // Reuse the per-step sensor query for golems, panic and the interaction partner.
        let nearby = self.nearby_entities();
        if scan_golems {
            self.golem_scan_ticks.store(GOLEM_SCAN_RATE, Relaxed);
            self.check_for_nearby_golem(&nearby);
        }
        if !panic_check && !social_check {
            return;
        }
        let panicking = self.is_panicking(&nearby);
        // NearestLivingEntitySensor updates panic input at the sensor cadence, not per REST check.
        self.sensed_panic.store(panicking, Relaxed);
        if panic_check && panicking {
            // VillagerPanicTrigger.tick requires three villagers that themselves want a golem.
            self.spawn_golem_if_needed(timestamp, PANIC_VILLAGERS_NEEDED, random);
        }
        if social_check {
            if panicking {
                self.clear_interaction_partner();
            } else {
                self.gossip_golem_check(timestamp, &nearby, random);
            }
        }
    }

    pub(super) fn is_panicking(&self, nearby: &[Arc<dyn EntityBase>]) -> bool {
        // VillagerPanicTrigger.isHurt / HurtBySensor use the remembered damage source.
        if self
            .mob_entity
            .living_entity
            .get_last_damage_type()
            .is_some()
        {
            return true;
        }
        let entity = self.get_entity();
        let world = entity.world.load();
        let range = self
            .mob_entity
            .living_entity
            .get_attribute_value(&Attributes::FOLLOW_RANGE);
        let visible = TargetPredicate::create_non_attackable().set_base_max_distance(range);
        nearby.iter().any(|other| {
            let Some(distance) = hostile_distance(other.get_entity().entity_type) else {
                return false;
            };
            entity
                .pos
                .load()
                .squared_distance_to_vec(&other.get_entity().pos.load())
                <= distance * distance
                && visible.test(&world, Some(self), other.as_ref())
        })
    }

    fn resolved_golem_activity(&self) -> Activity {
        let entity = self.get_entity();
        let scheduled = entity
            .world
            .load()
            .villager_activity(&entity.block_pos.load(), entity.age.load(Relaxed) < 0);
        // Villager's activity requirements -> Brain.setActiveActivityIfPossible defaults to IDLE.
        match scheduled {
            Activity::Work if self.get_job_site().is_none() => Activity::Idle,
            Activity::Meet
                if !self
                    .mob_entity
                    .brain
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .has_memory_value(types::MEETING_POINT.id()) =>
            {
                Activity::Idle
            }
            _ => scheduled,
        }
    }

    fn clear_interaction_partner(&self) {
        self.mob_entity
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .erase(types::INTERACTION_TARGET.id());
    }

    fn interaction_partner(&self, nearby: &[Arc<dyn EntityBase>]) -> Option<Arc<dyn EntityBase>> {
        let entity = self.get_entity();
        let valid = |other: &&Arc<dyn EntityBase>| {
            other.get_entity().entity_type == &EntityType::VILLAGER
                && other.get_entity().entity_id != entity.entity_id
                && other
                    .get_living_entity()
                    .is_some_and(LivingEntity::is_alive)
                && entity
                    .pos
                    .load()
                    .squared_distance_to_vec(&other.get_entity().pos.load())
                    <= INTERACTION_RANGE * INTERACTION_RANGE
                && self.has_line_of_sight(other.get_entity())
        };
        let remembered = self
            .mob_entity
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(types::INTERACTION_TARGET)
            .cloned();
        // InteractWith.of selects the closest visible villager; retain it until invalid or stopped.
        let target = nearby
            .iter()
            .filter(valid)
            .min_by_key(|other| {
                let is_remembered = remembered.as_ref().is_some_and(|target| {
                    target.get_entity().entity_id == other.get_entity().entity_id
                });
                let distance = entity
                    .pos
                    .load()
                    .squared_distance_to_vec(&other.get_entity().pos.load());
                (!is_remembered, ordered_float::OrderedFloat(distance))
            })
            .cloned();
        self.mob_entity
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_optional(types::INTERACTION_TARGET, target.clone());
        target
    }

    fn gossip_golem_check(
        &self,
        timestamp: i64,
        nearby: &[Arc<dyn EntityBase>],
        random: &mut impl rand::Rng,
    ) {
        if self.get_entity().age.load(Relaxed) < 0
            || self.get_entity().pose.load() == EntityPose::Sleeping
            || self.is_trading.load(Relaxed)
            || !matches!(
                self.resolved_golem_activity(),
                Activity::Idle | Activity::Meet
            )
        {
            // TradeWithVillager.stop erases INTERACTION_TARGET when the social behavior stops.
            self.clear_interaction_partner();
            return;
        }
        let Some(partner) = self.interaction_partner(nearby) else {
            return;
        };
        let Some(target) = partner.cast_any().downcast_ref::<Self>() else {
            return;
        };
        // TradeWithVillager.tick calls Villager.gossip within squared distance 5.
        let entity = self.get_entity();
        if gossip_ready(self.last_gossip_share_time.load(Relaxed), timestamp)
            && gossip_ready(target.last_gossip_share_time.load(Relaxed), timestamp)
            && target.get_entity().pose.load() != EntityPose::Sleeping
            && entity
                .pos
                .load()
                .squared_distance_to_vec(&target.get_entity().pos.load())
                <= GOSSIP_DISTANCE_SQ
        {
            self.last_gossip_share_time.store(timestamp, Relaxed);
            target.last_gossip_share_time.store(timestamp, Relaxed);
            // Villager.gossip uses five, unlike the panic call site.
            self.spawn_golem_if_needed(timestamp, GOSSIP_VILLAGERS_NEEDED, random);
        }
    }

    fn spawn_golem_if_needed(
        &self,
        timestamp: i64,
        villagers_needed: usize,
        random: &mut impl rand::Rng,
    ) {
        let entity = self.get_entity();
        let world = entity.world.load();
        let bounds = entity.bounding_box.load().expand(
            VILLAGER_SEARCH_RANGE,
            VILLAGER_SEARCH_RANGE,
            VILLAGER_SEARCH_RANGE,
        );
        let nearby = world.get_entities_at_box(&bounds);
        let villagers: Vec<_> = nearby
            .iter()
            .filter_map(|other| other.cast_any().downcast_ref::<Self>())
            .collect();
        // Pumpkin ticks entities in parallel. Overlapping groups must not summon concurrently;
        // try_lock also permits plugin spawn callbacks to reenter without blocking a tick worker.
        let mut reservations = Vec::with_capacity(villagers.len());
        for villager in &villagers {
            let Ok(guard) = villager.golem_spawn_lock.try_lock() else {
                return;
            };
            reservations.push(guard);
        }
        // Villager.spawnGolemIfNeeded counts only willing villagers, capped at five.
        if !self.wants_to_spawn_golem(timestamp)
            || villagers
                .iter()
                .filter(|villager| villager.wants_to_spawn_golem(timestamp))
                .take(GOSSIP_VILLAGERS_NEEDED)
                .count()
                < villagers_needed
        {
            return;
        }
        if try_spawn_mob_with_random(
            &EntityType::IRON_GOLEM,
            SpawnReason::MobSummoned,
            IronGolemEntity::new,
            &world,
            &entity.block_pos.load(),
            SPAWN_ATTEMPTS,
            SPAWN_RANGE_XZ,
            SPAWN_RANGE_Y,
            SpawnStrategy::OnTopOfColliderNoLeaves,
            true,
            random,
        )
        .is_some()
        {
            for villager in villagers {
                villager.golem_detected();
            }
        }
        drop(reservations);
    }
}

const fn gossip_ready(last: i64, timestamp: i64) -> bool {
    timestamp < last || timestamp >= last + GOSSIP_INTERVAL
}

fn hostile_distance(entity_type: &EntityType) -> Option<f64> {
    // VillagerHostilesSensor.ACCEPTABLE_DISTANCE_FROM_HOSTILES (Java lines 11-22).
    match entity_type.resource_name {
        "drowned" | "husk" | "vex" | "zombie" | "zombie_villager" => Some(8.0),
        "evoker" | "illusioner" | "ravager" => Some(12.0),
        "pillager" => Some(15.0),
        "vindicator" | "zoglin" => Some(10.0),
        _ => None,
    }
}
