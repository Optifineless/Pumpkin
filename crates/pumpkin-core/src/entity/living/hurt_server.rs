use super::LivingEntity;
use crate::entity::EntityBase;
use crate::entity::player::statistics::{CustomStatistic, StatisticCategory};
use pumpkin_data::damage::DamageType;
use pumpkin_data::tag::{self, Taggable};
use std::sync::atomic::Ordering::Relaxed;

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "Regression tests require valid fixtures and locks"
)]
mod tests;

impl LivingEntity {
    // LivingEntity.hurtServer compares damage after blocking, before armor/magic absorption.
    pub(super) fn damage_after_cooldown(
        &self,
        damage: f32,
        damage_type: &DamageType,
    ) -> Option<(f32, bool)> {
        let last_damage = self
            .damage_owner
            .admission_baseline(self.last_damage_taken.load());
        let took_full_damage = self.hurt_cooldown.load(Relaxed) <= 10
            || damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_COOLDOWN);
        let admitted = if took_full_damage {
            self.hurt_cooldown.store(20, Relaxed);
            damage
        } else {
            if damage <= last_damage {
                return None;
            }
            damage - last_damage
        };
        if took_full_damage {
            self.last_damage_taken.store(damage);
        }
        Some((admitted.max(0.0), took_full_damage))
    }

    // LivingEntity.actuallyHurt / Player.actuallyHurt: callbacks split serial mutation segments.
    pub(super) fn actually_hurt(
        &self,
        caller: &dyn EntityBase,
        incoming: f32,
        full_hit: bool,
        context: super::damage::HurtContext<'_>,
    ) -> bool {
        let super::damage::HurtContext {
            damage_type,
            source,
            cause,
            ..
        } = context;
        if !self.valid_hurt_context(caller, context) {
            return false;
        }
        // LivingEntity.actuallyHurt:1961 / Player.actuallyHurt:738: one immunity roll,
        // before armor and absorption. Revalidation must never reroll random predicates.
        if self.is_immune_to_enchantment_damage(caller, damage_type, source, cause) {
            return true;
        }
        let remaining = || {
            if full_hit {
                incoming
            } else {
                (incoming
                    - self
                        .damage_owner
                        .admission_baseline(self.last_damage_taken.load()))
                .max(0.0)
            }
        };
        let weapon = Self::damage_source_weapon(source);
        if !damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_ARMOR) {
            self.hurt_armor(caller, &damage_type, remaining());
        }
        // Armor callbacks may heal, kill, reset, change abilities, or admit a stronger hit.
        if !self.valid_hurt_context(caller, context) {
            return false;
        }
        let revision = self.damage_owner.revision();
        let damage = self.reduce_armor_damage(remaining(), &damage_type, weapon.as_ref());
        let reduced = self.get_damage_after_magic_absorb(damage, &damage_type, caller, cause);
        let damage = if revision == self.damage_owner.revision() {
            reduced
        } else {
            // Recompute pure reductions, without replaying equipment or statistic callbacks.
            self.reduce_magic_damage(
                self.reduce_armor_damage(remaining(), &damage_type, weapon.as_ref()),
                &damage_type,
                None,
                None,
            )
        };
        if !self.valid_hurt_context(caller, context) {
            return false;
        }
        self.damage_owner.reserve_admission(incoming);
        self.apply_health_damage(caller, context, damage)
    }

    // The absorption/health portion of LivingEntity.actuallyHurt and Player.actuallyHurt.
    fn apply_health_damage(
        &self,
        caller: &dyn EntityBase,
        context: super::damage::HurtContext<'_>,
        damage: f32,
    ) -> bool {
        let super::damage::HurtContext {
            damage_type,
            source,
            cause,
            ..
        } = context;
        let mut health_damage = self.consume_damage_absorption(damage);
        let absorbed = damage - health_damage;
        let revision = self.damage_owner.revision();
        Self::award_absorbed_damage(caller, cause, absorbed);
        if !self.valid_hurt_context(caller, context) {
            return false;
        }
        let mut extra_absorbed = 0.0;
        if revision != self.damage_owner.revision() {
            let remaining = self.consume_damage_absorption(health_damage);
            extra_absorbed += health_damage - remaining;
            health_damage = remaining;
        }
        let revision = self.damage_owner.revision();
        if health_damage > 0.0 {
            if let Some(player) = caller.get_player()
                && damage_type.exhaustion > 0.0
            {
                player.add_exhaustion(damage_type.exhaustion);
            }
            if !self.valid_hurt_context(caller, context) {
                return false;
            }
            if revision != self.damage_owner.revision() {
                // Newly granted absorption is live state too; never overwrite it with a snapshot.
                let remaining = self.consume_damage_absorption(health_damage);
                extra_absorbed += health_damage - remaining;
                health_damage = remaining;
            }
            if !self.valid_hurt_context(caller, context) {
                return false;
            }
            self.record_health_damage(damage_type, health_damage, source, cause);
            self.set_health(self.health.load() - health_damage);
            if health_damage < f32::MAX / 10.0
                && let Some(player) = caller.get_player()
            {
                player.increment_stat(
                    StatisticCategory::Custom,
                    CustomStatistic::DamageTaken as i32,
                    (health_damage * 10.0).round() as i32,
                );
            }
            if self.damage_owner.lifecycle() != context.lifecycle {
                return false;
            }
            if caller.get_player().is_none() {
                self.set_absorption((self.absorption.load() - health_damage).max(0.0));
            }
            if health_damage > 0.0 {
                self.entity
                    .world
                    .load()
                    .emit_game_event("entity_damage", self.entity.pos.load());
            }
        }
        Self::award_absorbed_damage(caller, cause, extra_absorbed);
        true
    }

    fn consume_damage_absorption(&self, damage: f32) -> f32 {
        let absorption = self.absorption.load();
        let health_damage = (damage - absorption).max(0.0);
        self.set_absorption((absorption - (damage - health_damage)).max(0.0));
        health_damage
    }

    fn award_absorbed_damage(
        caller: &dyn EntityBase,
        cause: Option<&dyn EntityBase>,
        absorbed: f32,
    ) {
        if absorbed <= 0.0 || absorbed >= f32::MAX / 10.0 {
            return;
        }
        // Player.actuallyHurt awards the victim; base LivingEntity awards the attacker.
        let recipient = caller
            .get_player()
            .map(|p| (p, CustomStatistic::DamageAbsorbed))
            .or_else(|| {
                cause
                    .and_then(EntityBase::get_player)
                    .map(|p| (p, CustomStatistic::DamageDealtAbsorbed))
            });
        if let Some((player, statistic)) = recipient {
            player.increment_stat(
                StatisticCategory::Custom,
                statistic as i32,
                (absorbed * 10.0).round() as i32,
            );
        }
    }
}
