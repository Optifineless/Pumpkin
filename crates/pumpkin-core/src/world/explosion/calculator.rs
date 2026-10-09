use super::{Explosion, World};
use crate::entity::EntityBase;
use pumpkin_data::{
    Block,
    tag::{Tag, Taggable},
};
use pumpkin_util::math::position::BlockPos;

// ExplosionDamageCalculator.getEntityDamageAmount; distance is normalized by the diameter.
pub(super) fn entity_damage_amount(power: f32, distance: f64, exposure: f32) -> f32 {
    let impact = (1.0 - distance) * f64::from(exposure);
    (f64::midpoint(impact * impact, impact) * 7.0 * f64::from(power * 2.0) + 1.0) as f32
}

/// Defines how damage and block destruction are calculated for an explosion.
pub trait ExplosionDamageCalculator: Send + Sync {
    /// Returns the block's explosion resistance. If None, the block is treated as air/empty.
    fn get_block_explosion_resistance(
        &self,
        _explosion: &Explosion,
        _world: &World,
        _pos: &BlockPos,
        block: &Block,
        fluid: &pumpkin_data::fluid::FluidState,
    ) -> Option<f32> {
        if block.default_state.is_air() && fluid.is_empty {
            None
        } else {
            Some(fluid.blast_resistance.max(block.blast_resistance))
        }
    }

    /// Returns whether this block should be destroyed / affected by the explosion.
    fn should_block_explode(
        &self,
        _explosion: &Explosion,
        _world: &World,
        _pos: &BlockPos,
        _block: &Block,
        _power: f32,
    ) -> bool {
        true
    }

    /// Returns whether the entity should take damage from the explosion.
    fn should_damage_entity(&self, _explosion: &Explosion, _entity: &dyn EntityBase) -> bool {
        true
    }

    /// Returns knockback multiplier for the given entity (default 1.0).
    fn get_knockback_multiplier(&self, _entity: &dyn EntityBase) -> f32 {
        1.0
    }

    /// Calculates the damage amount to deal to the entity given the exposure.
    fn get_entity_damage_amount(
        &self,
        explosion: &Explosion,
        entity: &dyn EntityBase,
        exposure: f32,
    ) -> f32 {
        let radius = f64::from(explosion.power * 2.0);
        let distance = (entity
            .get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&explosion.pos))
        .sqrt()
            / radius;
        entity_damage_amount(explosion.power, distance, exposure)
    }
}

/// Default explosion damage calculator implementing vanilla standard explosion rules.
pub struct DefaultExplosionDamageCalculator;

impl ExplosionDamageCalculator for DefaultExplosionDamageCalculator {}

/// A configurable explosion damage calculator (e.g. for wind charges, mace wind bursts).
pub struct SimpleExplosionDamageCalculator {
    pub damages_entities: bool,
    pub damages_blocks: bool,
    pub knockback_multiplier: Option<f32>,
    pub immune_blocks: Option<&'static Tag>,
}

impl SimpleExplosionDamageCalculator {
    #[must_use]
    pub const fn new(
        damages_entities: bool,
        damages_blocks: bool,
        knockback_multiplier: Option<f32>,
        immune_blocks: Option<&'static Tag>,
    ) -> Self {
        Self {
            damages_entities,
            damages_blocks,
            knockback_multiplier,
            immune_blocks,
        }
    }
}

impl ExplosionDamageCalculator for SimpleExplosionDamageCalculator {
    fn get_block_explosion_resistance(
        &self,
        _explosion: &Explosion,
        _world: &World,
        _pos: &BlockPos,
        block: &Block,
        fluid: &pumpkin_data::fluid::FluidState,
    ) -> Option<f32> {
        // SimpleExplosionDamageCalculator: tagged blocks stop the ray; all others are transparent.
        if let Some(immune_tag) = self.immune_blocks {
            return block.has_tag(immune_tag).then_some(3_600_000.0);
        }
        if block.default_state.is_air() && fluid.is_empty {
            None
        } else {
            Some(fluid.blast_resistance.max(block.blast_resistance))
        }
    }

    fn should_block_explode(
        &self,
        _explosion: &Explosion,
        _world: &World,
        _pos: &BlockPos,
        _block: &Block,
        _power: f32,
    ) -> bool {
        self.damages_blocks
    }

    fn should_damage_entity(&self, _explosion: &Explosion, _entity: &dyn EntityBase) -> bool {
        self.damages_entities
    }

    fn get_knockback_multiplier(&self, entity: &dyn EntityBase) -> f32 {
        if entity
            .get_player()
            .is_some_and(crate::entity::player::Player::is_flying)
        {
            0.0
        } else {
            self.knockback_multiplier.unwrap_or(1.0)
        }
    }
}

/// `EntityBasedExplosionDamageCalculator` delegates the source's resistance override.
pub struct EntityBasedExplosionDamageCalculator {
    pub source: std::sync::Arc<dyn EntityBase>,
}
impl ExplosionDamageCalculator for EntityBasedExplosionDamageCalculator {
    fn get_block_explosion_resistance(
        &self,
        explosion: &Explosion,
        world: &World,
        pos: &BlockPos,
        block: &Block,
        fluid: &pumpkin_data::fluid::FluidState,
    ) -> Option<f32> {
        let resistance = DefaultExplosionDamageCalculator
            .get_block_explosion_resistance(explosion, world, pos, block, fluid)?;
        // WitherSkull.getBlockExplosionResistance / WitherBoss.canDestroy. Other sources use Entity's identity override.
        if self
            .source
            .cast_any()
            .downcast_ref::<crate::entity::projectile::wither_skull::WitherSkullEntity>()
            .is_some_and(crate::entity::projectile::wither_skull::WitherSkullEntity::is_dangerous)
            && !block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_WITHER_IMMUNE)
        {
            Some(resistance.min(0.8))
        } else {
            Some(resistance)
        }
    }
}

pub(super) struct RespawnAnchorDamageCalculator {
    pub origin: BlockPos,
    pub in_water: bool,
}
impl ExplosionDamageCalculator for RespawnAnchorDamageCalculator {
    fn get_block_explosion_resistance(
        &self,
        explosion: &Explosion,
        world: &World,
        pos: &BlockPos,
        block: &Block,
        fluid: &pumpkin_data::fluid::FluidState,
    ) -> Option<f32> {
        if self.in_water && *pos == self.origin {
            Some(Block::WATER.blast_resistance)
        } else {
            DefaultExplosionDamageCalculator
                .get_block_explosion_resistance(explosion, world, pos, block, fluid)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{Entity, living::test_support::armor_test_world};
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::vector3::Vector3;
    #[tokio::test]
    async fn tnt_damage_uses_diameter_and_keeps_the_obstructed_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let world = armor_test_world(dir.path());
        let explosion = Explosion::new(
            4.0,
            Vector3::default(),
            super::super::BlockInteraction::Destroy,
        );
        // Hand-derived from impacts 7/8, 3/4 and 1/2: 46.9375, 37.75, 22 health points.
        for (distance, expected) in [(1.0, 46.9375), (2.0, 37.75), (4.0, 22.0)] {
            let victim = Entity::new(
                world.clone(),
                Vector3::new(distance, 0.0, 0.0),
                &EntityType::COW,
            );
            assert_eq!(
                DefaultExplosionDamageCalculator.get_entity_damage_amount(&explosion, &victim, 1.0),
                expected
            );
            assert_eq!(
                DefaultExplosionDamageCalculator.get_entity_damage_amount(&explosion, &victim, 0.0),
                1.0
            );
        }
        assert_eq!(entity_damage_amount(3.0, 0.0, 1.0), 43.0);
        assert_eq!(entity_damage_amount(6.0, 0.0, 1.0), 85.0);
        assert!((entity_damage_amount(3.0, 1.0 / 6.0, 1.0) - 33.083_332).abs() < 0.0001);
        assert!((entity_damage_amount(6.0, 1.0 / 12.0, 1.0) - 74.791_664).abs() < 0.0001);
        crate::server::fixture_lifecycle::finish().await;
    }

    #[test]
    fn blast_knockback_applies_the_effective_resistance_after_exposure() {
        assert_eq!(
            super::super::knockback_power(0.125, 1.0, 1.0, 0.75),
            0.21875
        );
        assert_eq!(super::super::knockback_power(0.25, 1.0, 1.0, 0.0), 0.75);
        assert_eq!(super::super::knockback_power(0.5, 0.5, 1.0, 0.5), 0.125);
        assert_eq!(super::super::knockback_power(0.0, 1.0, 1.0, 1.0), 0.0);
    }
}
