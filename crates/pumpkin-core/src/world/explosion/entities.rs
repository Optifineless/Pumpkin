use super::{
    DefaultExplosionDamageCalculator, Explosion, ExplosionDamageCalculator, World, knockback_power,
};
use crate::{entity::EntityBase, net::ClientPlatform};
use pumpkin_data::{entity::EntityType, tag::Taggable};
use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};
use rustc_hash::FxHashMap;
use std::sync::Arc;

impl Explosion {
    pub(super) fn damage_entities(&self, world: &Arc<World>) -> FxHashMap<i32, Vector3<f64>> {
        let mut player_knockback = FxHashMap::default();
        // Explosion is too small
        if self.power < 1.0e-5 {
            return player_knockback;
        }

        let radius = f64::from(self.power * 2.0);
        let min_x = (self.pos.x - radius - 1.0).floor() as i32;
        let max_x = (self.pos.x + radius + 1.0).floor() as i32;
        let min_y = (self.pos.y - radius - 1.0).floor() as i32;
        let max_y = (self.pos.y + radius + 1.0).floor() as i32;
        let min_z = (self.pos.z - radius - 1.0).floor() as i32;
        let max_z = (self.pos.z + radius + 1.0).floor() as i32;

        let search_box = BoundingBox::new(
            Vector3::new(min_x as f64, min_y as f64, min_z as f64),
            Vector3::new(max_x as f64, max_y as f64, max_z as f64),
        );

        let entities = world.get_all_at_box(&search_box);

        let default_calc = DefaultExplosionDamageCalculator;
        let calc: &dyn ExplosionDamageCalculator = match &self.damage_calculator {
            Some(c) => c.as_ref(),
            None => &default_calc,
        };

        for entity_base in entities {
            if self.source.as_ref().is_some_and(|source| {
                source.get_entity().entity_id == entity_base.get_entity().entity_id
            }) || self.ignores_entity(world, entity_base.as_ref())
            {
                continue;
            }

            // Skip spectators (no damage, no knockback)
            if entity_base.is_spectator() {
                continue;
            }

            let entity = entity_base.get_entity();

            let distance = (entity.pos.load().squared_distance_to_vec(&self.pos)).sqrt() / radius;
            if distance > 1.0 {
                continue;
            }

            let should_damage = calc.should_damage_entity(self, entity_base.as_ref());
            let knockback_multiplier = calc.get_knockback_multiplier(entity_base.as_ref()) as f64;

            let exposure = if !should_damage && knockback_multiplier == 0.0 {
                0.0
            } else {
                Self::calculate_exposure(&self.pos, entity, world) as f64
            };

            if should_damage && self.teams_allow_damage(entity_base.as_ref()) {
                let damage =
                    calc.get_entity_damage_amount(self, entity_base.as_ref(), exposure as f32);
                self.hurt_from_explosion(entity_base.as_ref(), damage);
            }

            if let Some(knockback) = self.knockback_entity(
                entity_base.as_ref(),
                distance,
                exposure,
                knockback_multiplier,
            ) {
                player_knockback.insert(entity.entity_id, knockback);
            }
        }
        player_knockback
    }

    fn knockback_entity(
        &self,
        entity_base: &dyn EntityBase,
        distance: f64,
        exposure: f64,
        knockback_multiplier: f64,
    ) -> Option<Vector3<f64>> {
        let entity = entity_base.get_entity();
        // Calculate and apply knockback
        let dir_pos = if entity.entity_type == &EntityType::TNT {
            entity.pos.load()
        } else {
            entity.get_eye_pos()
        };
        let direction = (dir_pos - self.pos).normalize();

        // ServerExplosion.hurtEntities reads the effective attribute, including enchantments.
        let resistance = entity_base.get_living_entity().map_or(0.0, |living| {
            living.get_attribute_value(
                &pumpkin_data::attributes::Attributes::EXPLOSION_KNOCKBACK_RESISTANCE,
            )
        });
        let knockback_power = knockback_power(distance, exposure, knockback_multiplier, resistance);
        let knockback = direction * knockback_power;
        // ServerExplosion.hurtEntities transfers redirectable projectiles to the explosion cause.
        if entity
            .entity_type
            .has_tag(&pumpkin_data::tag::EntityType::MINECRAFT_REDIRECTABLE_PROJECTILE)
            && let Some(state) = entity_base.projectile_state()
        {
            state.set_owner(self.damage_cause().map(EntityBase::get_entity));
        }
        // AbstractWindCharge.push ignores impulses, but still accepts the ownership transfer.
        if [
            EntityType::WIND_CHARGE.id,
            EntityType::BREEZE_WIND_CHARGE.id,
        ]
        .contains(&entity.entity_type.id)
        {
            return None;
        }
        // Vanilla `ServerExplosion.hurtEntities`: creative flyers get no knockback.
        if entity_base
            .get_player()
            .is_some_and(|player| player.is_creative() && player.is_flying())
        {
            return None;
        }
        if entity_base
            .get_player()
            .is_some_and(|player| matches!(player.client.as_ref(), ClientPlatform::Java(_)))
        {
            // Java applies this impulse to its own current motion in handleExplosion.
            // Sending server velocity would replay previous client-authoritative launches.
            Some(knockback)
        } else {
            entity.add_velocity(knockback);
            None
        }
    }

    // ArmorStand.ignoreExplosion / BlockAttachedEntity.ignoreExplosion / ServerExplosion.shouldAffectBlocklikeEntities.
    fn ignores_entity(&self, world: &World, victim: &dyn EntityBase) -> bool {
        if victim.is_immune_to_explosion() {
            return true;
        }
        let kind = victim.get_entity().entity_type;
        let attached = [
            &EntityType::ITEM_FRAME,
            &EntityType::GLOW_ITEM_FRAME,
            &EntityType::PAINTING,
            &EntityType::LEASH_KNOT,
        ]
        .contains(&kind);
        if !attached && kind != &EntityType::ARMOR_STAND {
            return false;
        }
        let wind = self.source.as_ref().is_some_and(|source| {
            [&EntityType::WIND_CHARGE, &EntityType::BREEZE_WIND_CHARGE]
                .contains(&source.get_entity().entity_type)
        });
        let affects = !wind
            && (world.level_info.load().game_rules.mob_griefing
                || matches!(
                    self.block_interaction,
                    super::BlockInteraction::Destroy | super::BlockInteraction::DestroyWithDecay
                ));
        !affects
            || attached
                && self
                    .source
                    .as_ref()
                    .is_some_and(|source| source.get_entity().is_in_water())
            || victim
                .cast_any()
                .downcast_ref::<crate::entity::decoration::armor_stand::ArmorStandEntity>()
                .is_some_and(crate::entity::decoration::armor_stand::ArmorStandEntity::is_invisible)
    }
}
