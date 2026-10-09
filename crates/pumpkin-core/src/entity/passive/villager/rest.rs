use std::sync::{Arc, atomic::Ordering::Relaxed};

use pumpkin_data::{
    entity::EntityPose,
    environment_attribute::Activity,
    tag::{self, Taggable},
    tracked_data,
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

use super::VillagerEntity;
use crate::{
    block::blocks::{abstract_bed, bed::BedBlock},
    entity::{
        EntityBase,
        ai::{
            brain::memory::{GlobalPos, types},
            goal::{Controls, Goal, interact_with_door::close_doors},
        },
        mob::Mob,
    },
    world::home_poi::is_home,
};

// SleepInBed.COOLDOWN_AFTER_BEING_WOKEN.
const COOLDOWN_AFTER_BEING_WOKEN: i64 = 100;

impl VillagerEntity {
    pub(super) fn write_rest_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        // LivingEntity.addAdditionalSaveData: BlockPos.CODEC writes an int array.
        if self.get_entity().pose.load() == EntityPose::Sleeping
            && let Some(home) = self.get_home_pos()
        {
            nbt.put(
                "sleeping_pos",
                pumpkin_nbt::tag::NbtTag::IntArray(vec![home.0.x, home.0.y, home.0.z]),
            );
        }
    }

    pub(super) fn home_memory(&self) -> Option<GlobalPos> {
        self.mob_entity
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(types::HOME)
            .copied()
    }

    pub(super) fn read_rest_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        // LivingEntity.readAdditionalSaveData restores Brain first; migrate pre-Brain fork saves only.
        let sleeping = nbt.get_int_array("sleeping_pos").and_then(|coords| {
            if let [x, y, z] = coords {
                Some(BlockPos::new(*x, *y, *z))
            } else {
                None
            }
        });
        if self.home_memory().is_none() {
            let legacy = nbt
                .get_int("HomeX")
                .or_else(|| nbt.get_int("BedX"))
                .zip(nbt.get_int("HomeY").or_else(|| nbt.get_int("BedY")))
                .zip(nbt.get_int("HomeZ").or_else(|| nbt.get_int("BedZ")))
                .map(|((x, y), z)| BlockPos::new(x, y, z))
                .or(sleeping);
            if let Some(pos) = legacy.and_then(|pos| {
                pumpkin_data::dimension::Dimension::from_name(
                    self.get_entity().world.load().dimension.minecraft_name,
                )
                .map(|dimension| GlobalPos::new(dimension, pos))
            }) {
                self.mob_entity
                    .brain
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .set(types::HOME, pos);
            }
        }
        *self
            .home_pos
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = sleeping;
        self.get_entity()
            .set_synced_data(tracked_data::villager::SLEEPING_POS_ID, sleeping);
        if sleeping.is_some() {
            self.get_entity().set_pose(EntityPose::Sleeping);
        }
    }

    fn abandon_unreachable_home(&self) -> bool {
        // SetWalkTargetFromBlockMemory.create, configured by VillagerGoalPackages.getRestPackage.
        const TOO_LONG_UNREACHABLE_DURATION: i64 = 1200;
        let world = self.get_entity().world.load_full();
        let mut brain = self
            .mob_entity
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if brain
            .get(types::CANT_REACH_WALK_TARGET_SINCE)
            .is_none_or(|since| world.get_world_age() - since <= TOO_LONG_UNREACHABLE_DURATION)
        {
            return false;
        }
        let home = brain.get(types::HOME).copied();
        brain.erase(types::HOME.id());
        brain.set(types::CANT_REACH_WALK_TARGET_SINCE, world.get_world_age());
        drop(brain);
        if let Some(home) = home.filter(|home| home.dimension.id == world.dimension.id) {
            world.release_home(home.pos, self.get_entity().entity_uuid);
            self.home_acquisition
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .failed(home.pos, world.get_world_age(), &mut rand::rng());
        }
        true
    }

    pub(super) fn wants_to_sleep(&self) -> bool {
        let entity = self.get_entity();
        entity
            .world
            .load()
            .villager_activity(&entity.block_pos.load(), entity.age.load(Relaxed) < 0)
            == Activity::Rest
            && !self.sensed_panic.load(Relaxed)
            && !self.is_trading.load(Relaxed)
            && !entity.has_vehicle()
    }

    fn home_owner(&self) -> Option<std::sync::Weak<dyn EntityBase>> {
        let owner: Arc<dyn EntityBase> = self
            .self_weak
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()?
            .upgrade()?;
        Some(Arc::downgrade(&owner))
    }

    pub(super) fn update_home(&self) {
        // AcquirePoi.findPathToPois / PoiManager.take: claim only a reachable HOME with a free ticket.
        let world = self.get_entity().world.load_full();
        // The goal adapter observes Villager.die/releaseAllPois on the next mob tick.
        if self.mob_entity.living_entity.health.load() <= 0.0 {
            if let Some(home) = self.home_memory() {
                let home_world = if home.dimension.id == world.dimension.id {
                    Some(world)
                } else {
                    world.server.upgrade().and_then(|server| {
                        server
                            .worlds
                            .load()
                            .iter()
                            .find(|world| world.dimension.id == home.dimension.id)
                            .cloned()
                    })
                };
                if let Some(world) = home_world {
                    world.release_home(home.pos, self.get_entity().entity_uuid);
                }
            }
            self.mob_entity
                .brain
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .erase(types::HOME.id());
            return;
        }
        let Some(owner) = self.home_owner() else {
            return;
        };
        if let Some(home) = self.home_memory() {
            // SleepInBed.checkExtraStartConditions never interprets another dimension's HOME here.
            if home.dimension.id != world.dimension.id {
                return;
            }
            self.validate_home(home.pos, owner);
            return;
        }
        let timestamp = world.get_world_age();
        let mut random = rand::rng();
        let mut acquisition = self
            .home_acquisition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !acquisition.ready(timestamp, &mut random) {
            return;
        }
        let candidates = acquisition.candidates(
            world.available_homes(self.get_entity().block_pos.load()),
            timestamp,
            &mut random,
        );
        let mut navigation = self
            .mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // AcquirePoi.findPathToPois performs one search over the entire five-position batch.
        let path = navigation.create_path_to_targets(&self.mob_entity, &candidates, 1);
        self.acquire_home_from_path(path, &candidates, owner, &mut acquisition);
    }

    /// Applies a HOME path result; only unreachable paths schedule candidate retries.
    pub(super) fn acquire_home_from_path(
        &self,
        path: Option<crate::entity::ai::pathfinder::path::Path>,
        candidates: &[BlockPos],
        owner: std::sync::Weak<dyn EntityBase>,
        acquisition: &mut super::acquire_home::AcquireHome,
    ) {
        // AcquirePoi.create handles the path result before attempting to take its target's ticket.
        let world = self.get_entity().world.load_full();
        if let Some(path) = path.filter(crate::entity::ai::pathfinder::path::Path::can_reach) {
            if !world.claim_home(path.get_target(), owner) {
                return;
            }
            if let Some(dimension) =
                pumpkin_data::dimension::Dimension::from_name(world.dimension.minecraft_name)
            {
                self.mob_entity
                    .brain
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .set(types::HOME, GlobalPos::new(dimension, path.get_target()));
            }
            world.broadcast_entity_event(
                self.get_entity(),
                pumpkin_data::entity_status::EntityStatus::VillagerHappy,
                Some(pumpkin_protocol::bedrock::server::actor_event::ActorEventID::VillagerHappy),
            );
            acquisition.clear();
        } else {
            let timestamp = world.get_world_age();
            let mut random = rand::rng();
            for home in candidates {
                acquisition.failed(*home, timestamp, &mut random);
            }
        }
    }

    pub(super) fn rest_tick(&self, timestamp: i64) {
        // WakeUp.create / SleepInBed.checkExtraStartConditions and start.
        let entity = self.get_entity();
        let sleeping = entity.pose.load() == EntityPose::Sleeping;
        let home = if sleeping {
            *self
                .home_pos
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        } else {
            self.get_home_pos()
        };
        let Some(home) = home else {
            return;
        };
        let world = entity.world.load_full();
        if sleeping {
            if self.get_home_pos() != Some(home)
                || !self.wants_to_sleep()
                || world
                    .get_block_state_id_if_loaded(&home)
                    .is_some_and(|state| !is_home(state))
            {
                self.wake_up_if_sleeping_at(home);
            } else {
                entity.set_velocity(Vector3::default());
                self.mob_entity
                    .living_entity
                    .movement_input
                    .store(Vector3::default());
                self.mob_entity.living_entity.jumping.store(false, Relaxed);
            }
            return;
        }
        if !self.wants_to_sleep()
            || home
                .to_centered_f64()
                .squared_distance_to_vec(&entity.pos.load())
                >= 2.0f64.powi(2)
        {
            return;
        }
        let last_woken = self
            .mob_entity
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(types::LAST_WOKEN)
            .copied();
        if last_woken.is_some_and(|woken| {
            timestamp - woken > 0 && timestamp - woken < COOLDOWN_AFTER_BEING_WOKEN
        }) {
            return;
        }
        let (block, state) = world.get_block_and_state_id(&home);
        if !is_home(state)
            || !block.has_tag(&tag::Block::MINECRAFT_VILLAGERS_CAN_SLEEP_ON_BED)
            || pumpkin_data::block_properties::WhiteBedLikeProperties::from_state_id(state).occupied
        {
            return;
        }
        let Some(position) = abstract_bed::sleep_position(&world, home) else {
            return;
        };
        close_doors(self, None, None);
        // LivingEntity.startSleeping positions at the actual shape height and clears navigation/input.
        self.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
        entity.set_pos(position);
        BedBlock::set_occupied(true, &world, block, &home, state);
        *self
            .home_pos
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(home);
        entity.set_pose(EntityPose::Sleeping);
        entity.set_synced_data(tracked_data::villager::SLEEPING_POS_ID, Some(home));
        entity.set_velocity(Vector3::default());
        self.mob_entity
            .living_entity
            .movement_input
            .store(Vector3::default());
        self.mob_entity.living_entity.jumping.store(false, Relaxed);
        self.record_last_slept(timestamp);
    }

    /// Wakes this villager only when it is sleeping at the supplied bed head.
    pub(crate) fn wake_up_if_sleeping_at(&self, home: BlockPos) -> bool {
        // AbstractBedBlock.kickVillagerOutOfBed / LivingEntity.stopSleeping (upstream #3693).
        let entity = self.get_entity();
        if entity.pose.load() != EntityPose::Sleeping
            || *self
                .home_pos
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                != Some(home)
        {
            return false;
        }
        let world = entity.world.load_full();
        let (block, state) = world.get_block_and_state_id(&home);
        if is_home(state) {
            BedBlock::set_occupied(false, &world, block, &home, state);
        }
        *self
            .home_pos
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        entity.set_pose(EntityPose::Standing);
        entity.set_synced_data(tracked_data::villager::SLEEPING_POS_ID, None::<BlockPos>);
        entity.set_pos(abstract_bed::stand_up_position(&world, entity, home));
        let timestamp = world
            .level_time
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .world_age;
        self.mob_entity
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set(types::LAST_WOKEN, timestamp);
        true
    }
}

#[derive(Default)]
pub(super) struct SleepAtHomeGoal {
    target: Option<BlockPos>,
    remaining_cooldown: u8,
}

impl Goal for SleepAtHomeGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if self.remaining_cooldown > 0 {
            self.remaining_cooldown -= 1;
            return false;
        }
        let Some(villager) = mob.cast_any().downcast_ref::<VillagerEntity>() else {
            return false;
        };
        if !villager.wants_to_sleep() || villager.abandon_unreachable_home() {
            return false;
        }
        self.target = villager.get_home_pos();
        self.target.is_some()
    }
    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        mob.cast_any()
            .downcast_ref::<VillagerEntity>()
            .is_some_and(|villager| {
                villager.wants_to_sleep()
                    && villager.get_home_pos() == self.target
                    && (mob.get_entity().pose.load() == EntityPose::Sleeping
                        || !mob
                            .get_mob_entity()
                            .navigator
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .is_idle())
            })
    }
    fn start(&mut self, mob: &dyn Mob) {
        // VillagerGoalPackages.getRestPackage -> SetWalkTargetFromBlockMemory (speed 0.5, close enough 1).
        if mob.get_entity().pose.load() == EntityPose::Sleeping {
            return;
        }
        if let Some(target) = self.target {
            let entity = mob.get_mob_entity();
            let mut navigation = entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let path = navigation.create_path(entity, target.to_centered_f64(), 1);
            // MoveToTargetSink.tryComputePath keeps the first unreachable timestamp.
            let mut brain = entity
                .brain
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if path
                .as_ref()
                .is_some_and(crate::entity::ai::pathfinder::path::Path::can_reach)
            {
                brain.erase(types::CANT_REACH_WALK_TARGET_SINCE.id());
            } else if brain.get(types::CANT_REACH_WALK_TARGET_SINCE).is_none() {
                brain.set(
                    types::CANT_REACH_WALK_TARGET_SINCE,
                    mob.get_entity().world.load().get_world_age(),
                );
            }
            navigation.move_to_path(path, 0.5, &entity.living_entity);
        }
    }
    fn stop(&mut self, mob: &dyn Mob) {
        // MoveToTargetSink.stop bounds the cooldown before retrying an interrupted route.
        self.remaining_cooldown = rand::RngExt::random_range(&mut rand::rng(), 0..40);
        self.target = None;
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
    }
    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}
