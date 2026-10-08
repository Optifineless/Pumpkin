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
        let last_damage = self.last_damage_taken.load();
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
        self.last_damage_taken.store(damage);
        Some((admitted.max(0.0), took_full_damage))
    }

    // LivingEntity.actuallyHurt / Player.actuallyHurt: reductions, combat entry, then health.
    pub(super) fn actually_hurt(
        &self,
        caller: &dyn EntityBase,
        damage: f32,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) {
        let damage_after_armor = self.get_damage_after_armor_absorb(damage, &damage_type, source);
        let damage_amount = self.get_damage_after_magic_absorb(
            damage_after_armor,
            &damage_type,
            caller,
            cause.or(source),
        );
        let original_damage = damage_amount;
        let current_abs = self.absorption.load();
        let dmg_to_health = (original_damage - current_abs).max(0.0);
        let absorbed_damage = original_damage - dmg_to_health;

        if absorbed_damage > 0.0 {
            let new_abs = (current_abs - absorbed_damage).max(0.0);
            self.set_absorption(new_abs);

            if let Some(player) = caller.get_player() {
                player.increment_stat(
                    StatisticCategory::Custom,
                    CustomStatistic::DamageAbsorbed as i32,
                    (absorbed_damage * 10.0).round() as i32,
                );
            }

            if let Some(attacker_player) = cause.or(source).and_then(|c| c.get_player()) {
                attacker_player.increment_stat(
                    StatisticCategory::Custom,
                    CustomStatistic::DamageDealtAbsorbed as i32,
                    (absorbed_damage * 10.0).round() as i32,
                );
            }
        }

        if dmg_to_health > 0.0 {
            if let Some(player) = caller.get_player()
                && damage_type.exhaustion > 0.0
            {
                player.add_exhaustion(damage_type.exhaustion);
            }

            self.record_health_damage(damage_type, dmg_to_health, source, cause);
            self.set_health(self.health.load() - dmg_to_health);
            if let Some(player) = caller.get_player() {
                player.increment_stat(
                    StatisticCategory::Custom,
                    CustomStatistic::DamageTaken as i32,
                    (dmg_to_health * 10.0).round() as i32,
                );
            }

            if let Some(attacker_player) = cause.or(source).and_then(|c| c.get_player()) {
                attacker_player.increment_stat(
                    StatisticCategory::Custom,
                    CustomStatistic::DamageDealt as i32,
                    (dmg_to_health * 10.0).round() as i32,
                );
            }
        }
    }
}
