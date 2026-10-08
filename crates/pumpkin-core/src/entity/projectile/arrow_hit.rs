use super::{
    arrow::{ArrowEntity, ArrowPickup},
    deflection, potion_effects,
};
use crate::{
    enchantment::{EnchantmentHelper, post_attack::AttackEffectContext},
    entity::{EntityBase, projectile_deflection::ProjectileDeflectionType},
};
use pumpkin_data::{
    attributes::Attributes,
    damage::DamageType,
    data_component_impl::PotionDurationScaleImpl,
    entity::EntityType,
    sound::{Sound, SoundCategory},
};
use pumpkin_util::math::vector3::Vector3;
use std::{sync::Arc, sync::atomic::Ordering};

impl ArrowEntity {
    // AbstractArrow.onHitEntity: count piercing hits before damage; reverse rejected hits instead of discarding.
    pub(super) fn hit_entity(&self, target: &Arc<dyn EntityBase>, hit_pos: Vector3<f64>) {
        let entity = &self.entity;
        let pierce = self.pierce_level.load(Ordering::Relaxed);
        if pierce > 0 {
            let mut pierced = self
                .pierced_entities
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if pierced.len() > usize::from(pierce) {
                entity.remove();
                return;
            }
            pierced.push(target.get_entity().entity_id);
        }
        let velocity = entity.velocity.load();
        let owner = self.projectile_owner();
        let context = AttackEffectContext {
            attacker: owner.as_deref().or(Some(self)),
            damaging_entity: Some(self),
            damage_type: DamageType::ARROW,
        };
        // AbstractArrow.onHitEntity evaluates the saved weapon with this target and DamageSource.
        let base_damage = self
            .get_weapon_item()
            .map_or(self.get_base_damage(), |weapon| {
                f64::from(EnchantmentHelper::modify_projectile_value(
                    &weapon,
                    target.as_ref(),
                    context,
                    "minecraft:damage",
                    self.get_base_damage() as f32,
                ))
            });
        let mut damage = (f64::from(velocity.length() as f32) * base_damage)
            .clamp(0.0, f64::from(i32::MAX))
            .ceil() as i32;
        if self.is_critical.load(Ordering::Relaxed) {
            damage = damage.saturating_add(rand::random_range(0..damage / 2 + 2));
        }
        if let Some(living) = owner.as_deref().and_then(EntityBase::get_living_entity) {
            living.set_last_hurt_mob(target.as_ref());
        }
        let old_fire = target.get_entity().fire_ticks.load(Ordering::Relaxed);
        let enderman = target.get_entity().entity_type == &EntityType::ENDERMAN;
        if entity.is_on_fire() || self.is_flame.load(Ordering::Relaxed) {
            target.get_entity().set_on_fire_for(5.0);
        }
        let succeeded = super::damage::hurt_entity(
            target.as_ref(),
            damage as f32,
            DamageType::ARROW,
            self,
            owner.as_deref().or(Some(self)),
        );
        if succeeded {
            self.successful_hit(target.as_ref(), owner.as_deref(), velocity);
            entity.world.load().broadcast_to_chunk(
                entity.chunk_pos.load(),
                &pumpkin_protocol::java::client::play::CSoundEffect::new(
                    pumpkin_protocol::IdOr::Id(Sound::EntityArrowHit as u16),
                    SoundCategory::Neutral,
                    &hit_pos,
                    1.0,
                    1.2 / (rand::random::<f32>() * 0.2 + 0.9),
                    0,
                ),
            );
            if pierce == 0 {
                entity.remove();
            }
        } else if !enderman {
            target
                .get_entity()
                .fire_ticks
                .store(old_fire, Ordering::Relaxed);
            let owner_uuid = self.projectile.owner_uuid();
            deflection::deflect(
                self,
                ProjectileDeflectionType::Simple,
                Some(target.as_ref()),
                owner.as_deref(),
                false,
                Vector3::new(0.2, 0.2, 0.2),
            );
            self.projectile.set_owner_uuid(owner_uuid);
            self.has_hit.store(false, Ordering::Relaxed);
            if entity.velocity.load().length_squared() < 1.0e-7 {
                if self.pickup.load() == ArrowPickup::Allowed {
                    let stack = self
                        .item_stack
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone();
                    entity
                        .world
                        .load()
                        .drop_stack(&entity.block_pos.load(), stack);
                }
                entity.remove();
            }
        }
    }

    fn successful_hit(
        &self,
        target: &dyn EntityBase,
        owner: Option<&dyn EntityBase>,
        velocity: Vector3<f64>,
    ) {
        let Some(living) = target.get_living_entity() else {
            return;
        };
        // AbstractArrow.doKnockback, after accepted damage and with effective resistance.
        let punch = self.get_weapon_item().map_or(0.0, |weapon| {
            EnchantmentHelper::modify_projectile_value(
                &weapon,
                target,
                AttackEffectContext {
                    attacker: owner.or(Some(self)),
                    damaging_entity: Some(self),
                    damage_type: DamageType::ARROW,
                },
                "minecraft:knockback",
                0.0,
            )
        });
        if punch > 0.0 {
            let resistance =
                (1.0 - living.get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE)).max(0.0);
            let impulse = Vector3::new(velocity.x, 0.0, velocity.z).normalize()
                * (f64::from(punch) * 0.6 * resistance);
            if impulse.length_squared() > 0.0 {
                target
                    .get_entity()
                    .add_velocity(impulse + Vector3::new(0.0, 0.1, 0.0));
            }
        }
        super::damage::post_attack_with_item(
            target,
            DamageType::ARROW,
            self,
            owner,
            self.get_weapon_item(),
        );
        let item = self
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let scale = item
            .get_data_component::<PotionDurationScaleImpl>()
            .map_or(1.0, |component| component.scale);
        // Arrow.doPostHurtEffects adds even instantaneous effects to the living effect lifecycle.
        for (effect_type, duration, amplifier, ambient, show_particles, show_icon) in
            crate::item::potion::PotionContents::read_potion_effects(&item)
        {
            living.add_effect(pumpkin_data::potion::Effect {
                effect_type,
                duration: potion_effects::with_scaled_duration(duration, scale),
                amplifier,
                ambient,
                show_particles,
                show_icon,
                blend: false,
            });
        }
        if self.entity.entity_type == &EntityType::SPECTRAL_ARROW {
            living.add_effect(Self::spectral_glowing_effect());
        }
        // AbstractArrow.onHitEntity sends the hit marker to the firing player only.
        if target.get_player().is_some()
            && let Some(player) = owner.and_then(EntityBase::get_player)
            && target.get_entity().entity_id != player.get_entity().entity_id
            && !self.entity.silent.load(Ordering::Relaxed)
        {
            player.try_send_client_packet(&pumpkin_protocol::java::client::play::CGameEvent::new(
                pumpkin_protocol::java::client::play::GameEvent::ArrowHitPlayer,
                0.0,
            ));
        }
    }
}
