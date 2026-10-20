//! Vanilla 26.3 damage order (local decompiled source line numbers):
//! ServerPlayer.hurtServer 995-1006 and Player.hurtServer 672-701 gate `PvP`, abilities and
//! difficulty. LivingEntity.hurtServer 1178-1216 gates invulnerability/death/fire resistance,
//! wakes sleepers, blocks, amplifies freezing, wears/scales helmets and normalizes damage.
//! Lines 1219-1233 admit a full hit or only the excess; actuallyHurt (1960-1977, Player
//! 737-759) wears armor, mitigates magic, consumes absorption, records combat before health,
//! and emits `ENTITY_DAMAGE`. Lines 1236-1252 resolve credit, dispatch blocked/damage feedback,
//! mark impact and apply default knockback/tilt. Lines 1255-1267 protect with a totem before
//! death or hurt sounds; 1269-1289 remember successful sources and trigger damage criteria.
//! walkAnimation.setSpeed(1.5) is client handleDamageEvent (2050-2063), via the damage packet.
//! Java explosion motion stays client-applied through Pumpkin's explosion packet; it must not
//! be overwritten by a pending server velocity packet.
//! Ownership spans serial segments; plugin dispatch releases it and resumed stages read live state.

use super::{
    LivingEntity,
    damage_transaction::{DamageToken, PendingHurt},
};
use crate::entity::{EntityBase, equipment_damage::EquippedItem};
use pumpkin_data::{
    damage::DamageType,
    data_component_impl::EquipmentSlot,
    effect::StatusEffect,
    tag::{self, Taggable},
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::sync::{Arc, atomic::Ordering::Relaxed};

#[derive(Clone, Copy)]
pub(super) struct HurtContext<'a> {
    pub lifecycle: u64,
    pub ender_pearl_transfer: bool,
    pub damage_type: DamageType,
    pub position: Option<Vector3<f64>>,
    pub source: Option<&'a dyn EntityBase>,
    pub cause: Option<&'a dyn EntityBase>,
    pub fall_world: Option<&'a crate::world::World>,
}

struct HurtRequest<'a, F> {
    caller: &'a dyn EntityBase,
    amount: f32,
    damage_type: DamageType,
    position: Option<Vector3<f64>>,
    source: Option<&'a dyn EntityBase>,
    cause: Option<&'a dyn EntityBase>,
    expected_lifecycle: Option<u64>,
    ender_pearl_transfer: bool,
    before_hurt: F,
}

struct AdmittedHurt<'a> {
    caller: &'a dyn EntityBase,
    amount: f32,
    context: HurtContext<'a>,
    fall_world: Option<Arc<crate::world::World>>,
    // Struct fields drop in order: retire admission before releasing the serial owner.
    _pending: PendingHurt,
    _owner: DamageToken,
}

impl<'a, F: FnOnce()> HurtRequest<'a, F> {
    fn admit(self, living: &LivingEntity) -> Option<AdmittedHurt<'a>> {
        let owner = living.damage_owner.enter();
        let lifecycle = living.damage_owner.lifecycle();
        if self
            .expected_lifecycle
            .is_some_and(|expected| expected != lifecycle)
        {
            return None;
        }
        let pending = living.damage_owner.pending_hurt();
        (self.before_hurt)();
        Some(AdmittedHurt {
            caller: self.caller,
            amount: self.amount,
            fall_world: self
                .damage_type
                .has_tag(&tag::DamageType::MINECRAFT_IS_FALL)
                .then(|| living.entity.world.load_full()),
            context: HurtContext {
                lifecycle,
                ender_pearl_transfer: self.ender_pearl_transfer,
                damage_type: self.damage_type,
                position: self.position,
                source: self.source,
                cause: self.cause,
                fall_world: None,
            },
            _owner: owner,
            _pending: pending,
        })
    }
}

#[cfg(test)]
mod boundary_tests;
#[cfg(test)]
mod callback_tests;
#[cfg(test)]
mod instant_tests;
#[cfg(test)]
mod owner_exit_tests;
#[cfg(test)]
mod ownership_immunity_tests;
#[cfg(test)]
mod ownership_tests;
#[cfg(test)]
mod review3_tests;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod writer_tests;

impl LivingEntity {
    pub(super) fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        self.hurt_server_inner(HurtRequest {
            caller,
            amount,
            damage_type,
            position,
            source,
            cause,
            expected_lifecycle: None,
            ender_pearl_transfer: false,
            before_hurt: || {},
        })
    }

    // Carry the supporting block's original life into ordinary damage admission.
    pub(super) fn hurt_fall_for_context(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        expected: &super::FallContext,
    ) -> bool {
        let _owner = self.own_damage();
        if !expected.is_current(self, caller) {
            return false;
        }
        self.hurt_server_inner(HurtRequest {
            caller,
            amount,
            damage_type,
            position: None,
            source: None,
            cause: None,
            expected_lifecycle: Some(expected.lifecycle),
            ender_pearl_transfer: false,
            before_hurt: || {},
        })
    }

    /// Resets pearl fall state and damages only the captured life under damage admission.
    pub(crate) fn hurt_ender_pearl_for_lifecycle(
        &self,
        caller: &dyn EntityBase,
        lifecycle: u64,
        was_loaded: bool,
    ) -> bool {
        // ThrownEnderpearl.onHit resets fall distance and applies 5 damage after ServerPlayer.teleport.
        self.hurt_server_inner(HurtRequest {
            caller,
            amount: 5.0,
            damage_type: DamageType::ENDER_PEARL,
            position: None,
            source: None,
            cause: None,
            expected_lifecycle: Some(lifecycle),
            ender_pearl_transfer: was_loaded,
            before_hurt: || {
                if self.is_alive() {
                    self.reset_fall_distance_and_impulse_context();
                }
            },
        })
    }

    fn hurt_server_inner<F>(&self, request: HurtRequest<'_, F>) -> bool
    where
        F: FnOnce(),
    {
        let Some(request) = request.admit(self) else {
            return false;
        };
        let caller = request.caller;
        let context = HurtContext {
            fall_world: request.fall_world.as_deref(),
            ..request.context
        };
        let damage_type = context.damage_type;
        let position = context.position;
        let source = context.source;
        let cause = context.cause;
        // LivingEntity.hurtServer: admit this life before any Pumpkin plugin dispatch.
        let amount = if let Some(player) = caller.get_player() {
            let Some(amount) = player.admit_incoming_damage(
                request.amount,
                damage_type,
                cause,
                source,
                context.ender_pearl_transfer,
            ) else {
                return false;
            };
            amount
        } else {
            request.amount
        };
        // LivingEntity.hurtServer:1179 rolls before the death/fire-resistance gates.
        if self.entity.is_invulnerable_to(&damage_type, cause)
            || self.is_immune_to_enchantment_damage(caller, damage_type, source, cause)
            || !self.valid_hurt_context(caller, context)
        {
            return false;
        }
        let Some(amount) = self.damage_after_plugin_events(caller, amount, context) else {
            return false;
        };
        // An unlocked callback or a competing attacker may have killed or changed the victim.
        if !self.valid_hurt_context(caller, context) {
            return false;
        }
        if let Some(player) = caller.get_player()
            && player.sleeping_since.load().is_some()
        {
            player.wake_up();
        }
        // LivingEntity.hurtServer:1191-1211 resumes each serial stage on the admitted life.
        if !self.valid_hurt_context(caller, context) {
            return false;
        }
        self.no_action_time.store(0, Relaxed); // LivingEntity.hurtServer:1196.
        let original_damage = if amount < 0.0 { 0.0 } else { amount };
        let blocking_item = self.get_item_blocking_with();
        let blocked =
            self.apply_item_blocking(caller, &damage_type, original_damage, position, source);
        if !self.valid_hurt_context(caller, context) {
            return false;
        }
        let amount = self.damage_before_cooldown(caller, original_damage - blocked, damage_type);
        if !self.valid_hurt_context(caller, context) {
            return false;
        }
        let Some((damage_amount, full_hit)) = self.damage_after_cooldown(amount, &damage_type)
        else {
            return false;
        };
        if !self.try_absorb_wolf_armor_damage(&damage_type, damage_amount)
            && !self.actually_hurt(caller, amount, full_hit, context)
        {
            return false;
        }
        if self.damage_owner.lifecycle() != context.lifecycle {
            return false;
        }
        if !full_hit {
            // LivingEntity.hurtServer:1224-1225 assigns lastHurt after actuallyHurt.
            self.last_damage_taken
                .store(amount.max(self.last_damage_taken.load()));
        }
        if full_hit {
            self.hurt_time.store(10, Relaxed); // LivingEntity.hurtServer: hurtDuration = 10.
        }
        self.record_hurt_by(damage_type, source, cause);
        if full_hit {
            self.full_hit_feedback(caller, context, blocked, amount, blocking_item.as_ref());
        }
        self.finish_hurt(caller, damage_type, source, cause, full_hit);
        if self.damage_owner.lifecycle() != context.lifecycle {
            return false;
        }
        let success = blocked <= 0.0 || amount > 0.0;
        if success {
            *self
                .last_damage_type
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(damage_type);
            self.last_damage_stamp
                .store(self.entity.world.load().get_world_age(), Relaxed);
            self.notify_effects_of_damage(damage_type, amount);
        }
        Self::damage_criteria_and_block_stat(caller, context, original_damage, amount, blocked);
        if let Some(player) = caller.get_player() {
            player.send_health();
        }
        success && self.damage_owner.lifecycle() == context.lifecycle
    }

    pub(super) fn valid_hurt_context(
        &self,
        caller: &dyn EntityBase,
        context: HurtContext<'_>,
    ) -> bool {
        self.damage_owner.lifecycle() == context.lifecycle
            && context.fall_world.is_none_or(|world| {
                self.fall_session_current(caller, world, context.ender_pearl_transfer)
            })
            && self.damage_owner.admits_health(self.health.load())
            && !self.rejects_damage(context.damage_type, context.cause)
            && caller.get_player().is_none_or(|player| {
                player
                    .prepare_incoming_damage(
                        1.0,
                        context.damage_type,
                        context.cause,
                        context.source,
                        context.ender_pearl_transfer,
                    )
                    .is_some()
            })
    }

    // LivingEntity.hurtServer:1274-1276 / MobEffectInstance.onMobHurt, including absorbed hits.
    fn notify_effects_of_damage(&self, damage_type: DamageType, damage: f32) {
        let effects: Vec<_> = self
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        for effect in effects {
            if let Some(behavior) = crate::entity::effect::get_mob_effect(effect.effect_type) {
                behavior.on_mob_hurt(self, effect.amplifier, &damage_type, damage);
            }
        }
    }

    pub(super) fn rejects_damage(
        &self,
        damage_type: DamageType,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        self.entity.is_invulnerable_to(&damage_type, cause)
            || self.health.load() <= 0.0
            || self.dead.load(Relaxed)
            || (damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FIRE)
                && self.has_effect(&StatusEffect::FIRE_RESISTANCE))
    }

    fn damage_before_cooldown(
        &self,
        caller: &dyn EntityBase,
        mut damage: f32,
        damage_type: DamageType,
    ) -> f32 {
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FREEZING)
            && self
                .entity
                .entity_type
                .has_tag(&tag::EntityType::MINECRAFT_FREEZE_HURTS_EXTRA_TYPES)
        {
            damage *= 5.0;
        }
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_DAMAGES_HELMET)
            && !EquippedItem::capture(caller, &EquipmentSlot::HEAD)
                .stack
                .is_empty()
        {
            self.hurt_helmet(caller, &damage_type, damage);
            damage *= 0.75; // LivingEntity.hurtServer:1210.
        }
        if damage.is_finite() { damage } else { f32::MAX }
    }

    fn damage_after_plugin_events(
        &self,
        caller: &dyn EntityBase,
        mut amount: f32,
        context: HurtContext<'_>,
    ) -> Option<f32> {
        let HurtContext {
            damage_type,
            position,
            source,
            cause,
            ..
        } = context;
        let mut damage_event =
            crate::plugin::api::events::entity::entity_damage::EntityDamageEvent::new(
                self.entity.entity_id,
                damage_type,
                amount,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut damage_event);
        }
        if damage_event.cancelled || !self.valid_hurt_context(caller, context) {
            return None;
        }
        amount = damage_event.damage;

        if let Some(damager) = source.or(cause) {
            let mut by_entity_event =
                crate::plugin::api::events::entity::entity_damage_by_entity::EntityDamageByEntityEvent {
                    entity_id: self.entity.entity_id,
                    damager_id: damager.get_entity().entity_id,
                    damage: amount,
                    cause: format!("{damage_type:?}"),
                    cancelled: false,
                };
            if let Some(server) = self.entity.world.load().server.upgrade() {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut by_entity_event);
            }
            if by_entity_event.cancelled || !self.valid_hurt_context(caller, context) {
                return None;
            }
            amount = by_entity_event.damage;
        } else if position.is_some()
            || matches!(
                damage_type,
                DamageType::CACTUS
                    | DamageType::SWEET_BERRY_BUSH
                    | DamageType::CAMPFIRE
                    | DamageType::HOT_FLOOR
                    | DamageType::STALAGMITE
            )
        {
            let damager_pos = position.map(|p| {
                BlockPos(Vector3::new(
                    p.x.floor() as i32,
                    p.y.floor() as i32,
                    p.z.floor() as i32,
                ))
            });
            let mut by_block_event =
                crate::plugin::api::events::entity::entity_damage_by_block::EntityDamageByBlockEvent {
                    entity_id: self.entity.entity_id,
                    damager_pos,
                    damage: amount,
                    cause: format!("{damage_type:?}"),
                    cancelled: false,
                };
            if let Some(server) = self.entity.world.load().server.upgrade() {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut by_block_event);
            }
            if by_block_event.cancelled || !self.valid_hurt_context(caller, context) {
                return None;
            }
            amount = by_block_event.damage;
        }

        Some(amount)
    }
}
