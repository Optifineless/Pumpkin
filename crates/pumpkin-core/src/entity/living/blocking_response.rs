//! LivingEntity.blockUsingItem/blockedByItem and the hoglin, zoglin and ravager overrides.
use super::LivingEntity;
use crate::entity::{
    EntityBase,
    mob::{hoglin::HoglinEntity, ravager::RavagerEntity, zoglin::ZoglinEntity},
};
use pumpkin_data::{
    attributes::Attributes,
    damage::DamageType,
    entity::{EntityStatus, EntityType},
    sound::{Sound, SoundCategory},
    tag::{self, Taggable},
};
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;
use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

// Ravager.java:53-63.
const BASE_MOVEMENT_SPEED: f64 = 0.3;
const ATTACK_MOVEMENT_SPEED: f64 = 0.35;
const STUN_DURATION: i32 = 40;

impl LivingEntity {
    pub(super) fn block_using_item(
        &self,
        defender: &dyn EntityBase,
        attacker: &dyn EntityBase,
        fully_blocked: bool,
    ) {
        if let Some(hoglin) = attacker.cast_any().downcast_ref::<HoglinEntity>() {
            if !hoglin.is_baby.load(Relaxed) {
                self.throw_blocked_target(attacker);
            }
        } else if let Some(zoglin) = attacker.cast_any().downcast_ref::<ZoglinEntity>() {
            if !zoglin.is_baby() {
                self.throw_blocked_target(attacker);
            }
        } else if let Some(ravager) = attacker.cast_any().downcast_ref::<RavagerEntity>() {
            ravager.blocked_by_item(defender, rand::random::<f64>() < 0.5);
        } else if !fully_blocked {
            // LivingEntity.blockedByItem: this is separate from the later default 0.4 knockback.
            let delta = self.entity.pos.load() - attacker.get_entity().pos.load();
            self.knockback(0.5, delta.x, delta.z);
        }
    }

    fn throw_blocked_target(&self, attacker: &dyn EntityBase) {
        // HoglinBase.throwTarget (38-55): attributes, random angle in radians, additive push.
        let Some(living) = attacker.get_living_entity() else {
            return;
        };
        let strength = living.get_attribute_value(&Attributes::ATTACK_KNOCKBACK)
            - self.get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE);
        if strength <= 0.0 {
            return;
        }
        let delta = self.entity.pos.load() - attacker.get_entity().pos.load();
        let mut random = rand::rng();
        let angle = random.random_range(0..21) as f32 - 10.0;
        let scale = strength * f64::from(random.random::<f32>() * 0.5 + 0.2);
        let horizontal = Vector3::new(delta.x, 0.0, delta.z);
        let horizontal = if horizontal.length() < f64::from(1.0e-5f32) {
            Vector3::default()
        } else {
            horizontal.normalize() * scale
        };
        let (sin, cos) = (
            f64::from(pumpkin_util::math::sin(angle)),
            f64::from(pumpkin_util::math::cos(angle)),
        );
        self.push_hurt(Vector3::new(
            horizontal.x * cos + horizontal.z * sin,
            strength * f64::from(random.random::<f32>()) * 0.5,
            horizontal.z * cos - horizontal.x * sin,
        ));
        self.mark_hurt();
    }
}

#[derive(Default)]
pub struct RavagerBlockState {
    pub(crate) stunned: AtomicI32,
    pub(crate) roar: AtomicI32,
}

impl RavagerEntity {
    pub(crate) fn blocked_by_item(&self, defender: &dyn EntityBase, stun: bool) {
        // Ravager.blockedByItem: full blocks still stun or throw, unless already roaring.
        let living = &self.mob_entity.living_entity;
        let owner = living.own_damage();
        if self.blocking.roar.load(Relaxed) != 0 {
            return;
        }
        if stun {
            self.blocking.stunned.store(STUN_DURATION, Relaxed);
            let entity = &living.entity;
            entity.world.load().play_sound_fine(
                Sound::EntityRavagerStunned,
                SoundCategory::Hostile,
                &entity.pos.load(),
                1.0,
                1.0,
            );
            entity
                .world
                .load()
                .send_entity_status(entity, EntityStatus::RavagerStunned, None);
        }
        drop(owner); // Never hold both the ravager's and the defender's ownership.
        if stun {
            defender.push(self);
        } else {
            self.strong_block_knockback(defender);
        }
        if let Some(living) = defender.get_living_entity() {
            living.mark_hurt();
        }
    }

    fn strong_block_knockback(&self, defender: &dyn EntityBase) {
        // Ravager.strongKnockback.
        let delta =
            defender.get_entity().pos.load() - self.mob_entity.living_entity.entity.pos.load();
        let distance = (delta.x * delta.x + delta.z * delta.z).max(0.001);
        let impulse = Vector3::new(delta.x / distance * 4.0, 0.2, delta.z / distance * 4.0);
        if let Some(living) = defender.get_living_entity() {
            living.push_hurt(impulse);
        } else {
            defender.get_entity().push_impulse(impulse);
        }
    }

    pub(crate) fn tick_block_response(&self) {
        // Ravager.aiStep: stun ends in a 20-tick roar; its attack is at roarTick == 10.
        let living = &self.mob_entity.living_entity;
        let _owner = living.own_damage();
        if living.entity.is_removed() || living.health.load() <= 0.0 {
            return;
        }
        let speed =
            if self.blocking.stunned.load(Relaxed) > 0 || self.blocking.roar.load(Relaxed) > 0 {
                0.0
            } else {
                let target = if self.mob_entity.get_target().is_some() {
                    ATTACK_MOVEMENT_SPEED
                } else {
                    BASE_MOVEMENT_SPEED
                };
                let current = living.get_attribute_base(&Attributes::MOVEMENT_SPEED);
                current + 0.1 * (target - current)
            };
        living.set_attribute_base(&Attributes::MOVEMENT_SPEED, speed);
        if self.blocking.roar.load(Relaxed) > 0 && self.blocking.roar.fetch_sub(1, Relaxed) == 11 {
            self.block_roar();
        }
        if self.blocking.stunned.load(Relaxed) > 0
            && self.blocking.stunned.fetch_sub(1, Relaxed) == 1
        {
            let entity = &living.entity;
            entity.world.load().play_sound_fine(
                Sound::EntityRavagerRoar,
                SoundCategory::Hostile,
                &entity.pos.load(),
                1.0,
                1.0,
            );
            self.blocking.roar.store(20, Relaxed);
        }
    }

    fn block_roar(&self) {
        // Ravager.roar and its two game-rule predicates; illagers avoid damage, players avoid push.
        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load();
        let griefing = world.level_info.load().game_rules.mob_griefing;
        for target in world.get_all_at_box(&entity.bounding_box.load().expand(4.0, 4.0, 4.0)) {
            let other = target.get_entity();
            let Some(living) = target.get_living_entity() else {
                continue;
            };
            if other.entity_type == &EntityType::RAVAGER
                || other.is_removed()
                || living.health.load() <= 0.0
                || (!griefing && other.entity_type == &EntityType::ARMOR_STAND)
            {
                continue;
            }
            if !other
                .entity_type
                .has_tag(&tag::EntityType::MINECRAFT_ILLAGER)
            {
                target.damage_with_context(
                    target.as_ref(),
                    6.0,
                    DamageType::MOB_ATTACK,
                    None,
                    Some(self),
                    Some(self),
                );
            }
            if target.get_player().is_none() {
                self.strong_block_knockback(target.as_ref());
            }
        }
        world.emit_game_event("entity_action", entity.pos.load());
        world.send_entity_status(entity, EntityStatus::RavagerRoared, None);
    }
}
