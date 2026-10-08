use super::{BlockFlags, Explosion};
use crate::{block::blocks::fire::FireBlockBase, entity::EntityBase, world::World};
use pumpkin_data::damage::DamageType;
use rand::RngExt;
use std::sync::Arc;

pub(super) struct EnchantmentSettings {
    source: Option<Arc<dyn EntityBase>>,
    damage_type: Option<DamageType>,
    create_fire: bool,
}

impl Explosion {
    /// Carries `ExplodeEffect`'s attribution, damage identity and fire setting.
    #[must_use]
    pub fn with_enchantment_settings(
        mut self,
        source: Option<Arc<dyn EntityBase>>,
        damage_type: Option<DamageType>,
        create_fire: bool,
    ) -> Self {
        self.enchantment_settings = Some(EnchantmentSettings {
            source,
            damage_type,
            create_fire,
        });
        self
    }

    pub(super) fn hurt_from_explosion(&self, victim: &dyn EntityBase, damage: f32) {
        if let Some(settings) = &self.enchantment_settings {
            let source = settings.source.as_deref();
            victim.damage_with_context(
                victim,
                damage,
                settings.damage_type.unwrap_or(DamageType::EXPLOSION),
                source.is_none().then_some(self.pos),
                source,
                source,
            );
        } else {
            victim
                .get_entity()
                .damage(victim, damage, DamageType::EXPLOSION);
        }
    }

    // ServerExplosion.createFire; generated fire states retain the existing block behavior.
    pub(super) fn enchantment_fire_positions(
        &self,
        world: &Arc<World>,
    ) -> Vec<pumpkin_util::math::position::BlockPos> {
        if self
            .enchantment_settings
            .as_ref()
            .is_some_and(|settings| settings.create_fire)
        {
            self.get_blocks_to_destroy(world).into_keys().collect()
        } else {
            Vec::new()
        }
    }

    pub(super) fn create_enchantment_fire(
        &self,
        world: &Arc<World>,
        positions: &[pumpkin_util::math::position::BlockPos],
    ) {
        if !self
            .enchantment_settings
            .as_ref()
            .is_some_and(|settings| settings.create_fire)
        {
            return;
        }
        for pos in positions {
            if rand::rng().random_range(0..3) == 0
                && world.get_block_state(pos).is_air()
                && world.get_block_state(&pos.down()).is_solid_render()
            {
                let block = FireBlockBase::get_fire_type(world, pos);
                let state_id = if block.id == pumpkin_data::Block::FIRE.id {
                    crate::block::blocks::fire::fire::FireBlock.get_state_for_position(
                        world.as_ref(),
                        &block,
                        pos,
                    )
                } else {
                    block.default_state.id
                };
                world.set_block_state(pos, state_id, BlockFlags::NOTIFY_ALL);
            }
        }
    }
}
