//! Dolphin air seeking and jumping, from `BreathAirGoal` and `DolphinJumpGoal`.
use super::dolphin::DolphinEntity;
use crate::entity::{
    EntityBase,
    ai::{
        goal::try_find_water::TryFindWaterGoal,
        goal::{Controls, Goal, to_goal_ticks},
    },
    mob::Mob,
};
use pumpkin_data::{
    Block,
    damage::DamageType,
    effect::StatusEffect,
    entity::EntityStatus,
    sound::{Sound, SoundCategory},
    tag::{self, Taggable},
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::sync::{PoisonError, atomic::Ordering::Relaxed};

// Dolphin.getMaxAirSupply.
pub const MAX_AIR_SUPPLY: i32 = 4800;
impl DolphinEntity {
    pub(super) fn tick_air_supply(&self) {
        let entity = self.get_entity();
        let old = self.air_supply.load(Relaxed);
        let living = &self.mob_entity.living_entity;
        let world = entity.world.load();
        let eye = entity.pos.load() + Vector3::new(0.0, entity.get_eye_height(), 0.0);
        let submerged = entity.is_submerged_in_water()
            && world.get_block(&BlockPos::floored(eye.x, eye.y, eye.z)) != &Block::BUBBLE_COLUMN;
        let breathing = living.has_effect(&StatusEffect::WATER_BREATHING)
            || living.has_effect(&StatusEffect::CONDUIT_POWER);
        let nautilus_breath = living.has_effect(&StatusEffect::BREATH_OF_THE_NAUTILUS);
        // LivingEntity.baseTick/decreaseAirSupply and Dolphin.increaseAirSupply.
        let mut air = if self.mob_entity.is_no_ai() || !submerged || breathing {
            MAX_AIR_SUPPLY
        } else if nautilus_breath {
            old
        } else {
            let bonus =
                living.get_attribute_value(&pumpkin_data::attributes::Attributes::OXYGEN_BONUS);
            if bonus > 0.0 && rand::random::<f64>() >= 1.0 / (bonus + 1.0) {
                old
            } else {
                old - 1
            }
        };
        if air == old {
            return;
        }
        if let Some(server) = world.server.upgrade() {
            let mut event =
                crate::plugin::api::events::entity::entity_air_change::EntityAirChangeEvent::new(
                    entity.entity_id,
                    air,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
            air = event.amount;
        }
        if air <= -20 {
            air = 0;
            world.send_entity_status(entity, EntityStatus::DrownParticles, None);
            self.damage(self, 2.0, DamageType::DROWN);
        }
        self.air_supply.store(air, Relaxed);
        entity.set_synced_data(
            pumpkin_data::tracked_data::entity::DATA_AIR_SUPPLY_ID,
            pumpkin_protocol::codec::var_int::VarInt(air),
        );
    }
}

pub struct DolphinBreathAirGoal;
impl DolphinBreathAirGoal {
    fn find_air(mob: &dyn Mob) {
        let entity = mob.get_entity();
        let world = entity.world.load();
        let pos = entity.block_pos.load();
        // BlockPos.neighborColumn searches the center column first, then N/E/S/W.
        let destination = [(0, 0), (0, -1), (1, 0), (0, 1), (-1, 0)]
            .into_iter()
            .find_map(|(x, z)| {
                (pos.0.y..=pos.0.y + 8).find_map(|y| {
                    let at = BlockPos::new(pos.0.x + x, y, pos.0.z + z);
                    let (block, state) = world.get_block_and_state(&at);
                    let gives_air = (world.get_fluid(&at) == &pumpkin_data::fluid::Fluid::EMPTY
                        || block == &Block::BUBBLE_COLUMN)
                        && world.block_registry.is_pathfindable(
                            block,
                            state,
                            crate::entity::ai::pathfinder::node::PathComputationType::Land,
                        );
                    gives_air.then_some(at)
                })
            })
            .unwrap_or(BlockPos::new(pos.0.x, pos.0.y + 8, pos.0.z));
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .move_to_coords(
                f64::from(destination.0.x),
                f64::from(destination.0.y + 1),
                f64::from(destination.0.z),
                1.0,
                &mob.get_mob_entity().living_entity,
            );
    }
}
impl Goal for DolphinBreathAirGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.cast_any()
            .downcast_ref::<DolphinEntity>()
            .is_some_and(|d| d.air_supply.load(Relaxed) < 140)
    }
    fn can_stop(&self) -> bool {
        false
    }
    fn start(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
        Self::find_air(mob);
    }
    fn tick(&mut self, mob: &dyn Mob) {
        Self::find_air(mob);
        let entity = mob.get_entity();
        entity.update_velocity_from_input(
            mob.get_mob_entity().living_entity.movement_input.load(),
            f64::from(0.02f32),
        );
        entity.move_entity(mob, entity.velocity.load());
    }
    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}

#[derive(Default)]
pub struct DolphinJumpGoal {
    breached: bool,
}
fn motion_step(mob: &dyn Mob) -> (i32, i32) {
    use pumpkin_data::block_properties::HorizontalFacing;
    match mob.get_entity().get_horizontal_facing() {
        HorizontalFacing::North => (0, -1),
        HorizontalFacing::South => (0, 1),
        HorizontalFacing::East => (1, 0),
        HorizontalFacing::West => (-1, 0),
    }
}
impl Goal for DolphinJumpGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if rand::random_range(0..to_goal_ticks(10)) != 0 {
            return false;
        }
        let entity = mob.get_entity();
        let world = entity.world.load();
        let pos = entity.block_pos.load();
        let (x, z) = motion_step(mob);
        // DolphinJumpGoal.STEPS_TO_CHECK.
        [0, 1, 4, 5, 6, 7].into_iter().all(|step| {
            let at = BlockPos::new(pos.0.x + x * step, pos.0.y, pos.0.z + z * step);
            TryFindWaterGoal::is_water(&world, &at)
                && !world
                    .get_block(&at)
                    .has_tag(&tag::Block::MINECRAFT_BLOCKS_DOLPHIN_JUMP)
                && world.get_block_state(&at.up()).is_air()
                && world.get_block_state(&at.up().up()).is_air()
        })
    }
    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let e = mob.get_entity();
        let y = e.velocity.load().y;
        let pitch = e.pitch.load();
        (y * y >= f64::from(0.03f32)
            || pitch == 0.0
            || pitch.abs() >= 10.0
            || !e.touching_water.load(Relaxed))
            && !e.on_ground.load(Relaxed)
    }
    fn can_stop(&self) -> bool {
        false
    }
    fn start(&mut self, mob: &dyn Mob) {
        let (x, z) = motion_step(mob);
        let e = mob.get_entity();
        e.velocity
            .store(e.velocity.load() + Vector3::new(f64::from(x) * 0.6, 0.7, f64::from(z) * 0.6));
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
    }
    fn stop(&mut self, mob: &dyn Mob) {
        mob.get_entity().pitch.store(0.0);
    }
    fn tick(&mut self, mob: &dyn Mob) {
        let e = mob.get_entity();
        let previous = self.breached;
        if !previous {
            self.breached = TryFindWaterGoal::is_water(&e.world.load(), &e.block_pos.load());
        }
        if self.breached && !previous && !e.is_silent() {
            e.world.load().play_sound(
                Sound::EntityDolphinJump,
                SoundCategory::Neutral,
                &e.pos.load(),
            );
        }
        let velocity = e.velocity.load();
        if velocity.y * velocity.y < f64::from(0.03f32) && e.pitch.load() != 0.0 {
            e.pitch
                .store(e.pitch.load() + pumpkin_util::math::wrap_degrees(-e.pitch.load()) * 0.2);
        } else if velocity.length() > f64::from(1.0e-5f32) {
            e.pitch.store(
                (-velocity.y)
                    .atan2(velocity.x.hypot(velocity.z))
                    .to_degrees() as f32,
            );
        }
    }
    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::JUMP
    }
}
