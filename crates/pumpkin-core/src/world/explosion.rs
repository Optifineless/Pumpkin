mod block_triggers;
mod blocks;
mod calculator;
mod context;
#[cfg(test)]
mod context_tests;
mod enchantment;
mod entities;
mod exposure;
pub use calculator::*;

use std::sync::Arc;

use pumpkin_data::{Block, particle::Particle, sound::Sound, tag::Taggable};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use rustc_hash::FxHashMap;

use crate::entity::EntityBase;

use super::World;

/// Defines the type of explosion interaction with the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplosionInteraction {
    None,
    Block,
    Mob,
    Tnt,
    Trigger,
}

/// Defines how an explosion interacts with blocks in the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockInteraction {
    /// Keeps blocks intact (no block damage, no drops).
    Keep,
    /// Destroys blocks and drops 100% of items without decay.
    Destroy,
    /// Destroys blocks and applies loot decay based on explosion radius.
    DestroyWithDecay,
    /// Triggers block effects without destroying them.
    TriggerBlock,
}

/// Custom explosion damage attribution, independent of the direct entity's living owner.
#[derive(Clone)]
pub(crate) struct ExplosionDamageSource {
    pub(crate) cause: Option<Arc<dyn EntityBase>>,
}

pub struct Explosion {
    pub(super) power: f32,
    pub(super) pos: Vector3<f64>,
    block_interaction: BlockInteraction,
    damage_calculator: Option<Arc<dyn ExplosionDamageCalculator>>,
    preserve_rails: bool,
    source: Option<Arc<dyn EntityBase>>,
    cause: Option<Arc<dyn EntityBase>>,
    custom_cause: Option<ExplosionDamageSource>,
    damage_type: Option<pumpkin_data::damage::DamageType>,
    fire: bool,
    pub(super) small_particle: Particle,
    pub(super) large_particle: Particle,
    pub(super) sound: Sound,
}

pub struct ExplosionResult {
    pub block_count: u32,
    pub player_knockback: FxHashMap<i32, Vector3<f64>>,
    /// Admitted player lives used to discard stale impulses before delivery.
    pub player_lifecycles: FxHashMap<i32, u64>,
}

impl ExplosionResult {
    /// Returns a pending impulse only for the admitted player life, under its combat ownership.
    pub(crate) fn player_knockback_for(
        &self,
        player: &crate::entity::player::Player,
    ) -> Option<Vector3<f64>> {
        (self.player_lifecycles.get(&player.entity_id()).copied()
            == Some(player.living_entity.damage_lifecycle()))
        .then(|| self.player_knockback.get(&player.entity_id()).copied())
        .flatten()
    }
}

impl Explosion {
    #[must_use]
    pub const fn new(power: f32, pos: Vector3<f64>, block_interaction: BlockInteraction) -> Self {
        Self {
            power,
            pos,
            block_interaction,
            damage_calculator: None,
            preserve_rails: false,
            source: None,
            cause: None,
            custom_cause: None,
            damage_type: None,
            fire: false,
            small_particle: Particle::Explosion,
            large_particle: Particle::ExplosionEmitter,
            sound: Sound::EntityGenericExplode,
        }
    }

    #[must_use]
    pub fn with_damage_calculator(
        mut self,
        calculator: Arc<dyn ExplosionDamageCalculator>,
    ) -> Self {
        self.damage_calculator = Some(calculator);
        self
    }

    #[must_use]
    pub const fn preserving_rails(mut self) -> Self {
        self.preserve_rails = true;
        self
    }

    #[must_use]
    pub const fn with_particles_and_sound(
        mut self,
        small_particle: Particle,
        large_particle: Particle,
        sound: Sound,
    ) -> Self {
        self.small_particle = small_particle;
        self.large_particle = large_particle;
        self.sound = sound;
        self
    }

    fn protects_rail(&self, world: &World, pos: &BlockPos, block: &Block) -> bool {
        self.preserve_rails && (Self::is_rail(block) || Self::is_rail(world.get_block(&pos.up())))
    }

    fn is_rail(block: &Block) -> bool {
        block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_RAILS)
    }

    pub fn explode(&self, world: &Arc<World>) -> ExplosionResult {
        // ServerExplosion.explode calculates positions before entity damage and block callbacks.
        world.emit_game_event(
            pumpkin_data::game_event::GameEvent::Explode.name(),
            self.pos,
        );
        let positions = self.calculate_exploded_positions(world);
        let mut result = self.damage_entities(world);
        self.interact_with_blocks(world, &positions);
        self.create_fire(world, &positions);
        let block_count = positions.len() as u32;
        result.block_count = block_count;
        result
    }
}

// ServerExplosion.hurtEntities, before multiplying by the normalized direction.
fn knockback_power(distance: f64, exposure: f64, multiplier: f64, resistance: f64) -> f64 {
    (1.0 - distance) * exposure * multiplier * (1.0 - resistance)
}

#[cfg(test)]
mod tests {
    use super::{Explosion, World};
    use pumpkin_data::Block;
    use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

    #[test]
    fn exposure_ray_leaving_ground_is_not_obstructed() {
        let pos = BlockPos::new(8, 150, 8);
        let from = Vector3::new(8.5, 151.0, 8.5);
        assert!(!Explosion::clips_collision_shape(
            Block::STONE.default_state,
            &pos,
            from,
            Vector3::new(8.5, 151.25, 8.5),
        ));
        assert!(Explosion::clips_collision_shape(
            Block::STONE.default_state,
            &pos,
            from,
            Vector3::new(8.5, 150.75, 8.5),
        ));
    }

    #[test]
    fn collision_shape_extends_above_fence_outline() {
        let pos = BlockPos::new(0, 0, 0);
        let from = Vector3::new(0.0, 1.25, 0.5);
        let to = Vector3::new(1.0, 1.25, 0.5);
        assert!(Explosion::clips_collision_shape(
            Block::OAK_FENCE.default_state,
            &pos,
            from,
            to,
        ));
        assert!(!Explosion::clips_collision_shape(
            Block::SHORT_GRASS.default_state,
            &pos,
            Vector3::new(0.0, 0.5, 0.5),
            Vector3::new(1.0, 0.5, 0.5),
        ));
    }

    #[test]
    fn exposure_ray_only_checks_traversed_blocks_like_vanilla() {
        let fence_pos = BlockPos::new(0, 0, 0);
        // Official 26.3 BlockGetter.clip: the upper ray misses despite the
        // fence's 1.5-block collision height; only the lower ray visits it.
        for (height, expected_hit) in [(1.25, false), (0.75, true)] {
            let from = Vector3::new(-0.5, height, 0.5);
            let to = Vector3::new(1.5, height, 0.5);
            let hit = World::traverse_blocks(from, to, |pos, _| {
                let state = if *pos == fence_pos {
                    Block::OAK_FENCE.default_state
                } else {
                    Block::AIR.default_state
                };
                Explosion::clips_collision_shape(state, pos, from, to).then_some(())
            });
            assert_eq!(hit.is_some(), expected_hit, "ray at Y={height}");
        }
    }
}
