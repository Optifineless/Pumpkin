use super::{DamageToken, EntityBase, ItemStack, Ordering, Player, SpearItem, Vector3};
use crate::entity::combat;

#[cfg(test)]
#[path = "review3_tests.rs"]
mod review3_tests;

#[cfg(test)]
mod death_memory_tests;

impl SpearItem {
    // Player.stabAttack:1230-1231 runs both causeExtraKnockback segments before item effects.
    pub(super) fn stab_knockback(
        player: &Player,
        target: &dyn EntityBase,
        stack: &ItemStack,
        old_movement: Vector3<f64>,
        attack: Option<&DamageToken>,
    ) -> bool {
        let attacker = player.get_entity();
        combat::handle_knockback(attacker, target, 0.8);
        if !player.send_hurt_motion_owned(target, old_movement, false, attack) {
            return false;
        }
        let knockback_level = Self::knockback_level(stack);
        if knockback_level > 0 {
            combat::handle_knockback(attacker, target, f64::from(knockback_level));
        }
        // Player.causeExtraKnockback sends/restores only syncVelocity, cleared by the first call.
        if !player.send_hurt_motion_owned(target, old_movement, false, attack) {
            return false;
        }
        if target.get_player().is_some() {
            // LivingEntity.knockback only sets needsSync; ServerEntity.sendChanges excludes self.
            // Pumpkin skips player tracker motion, so consume this flag without sending to self.
            target
                .get_entity()
                .velocity_dirty
                .store(false, Ordering::Relaxed);
        }
        player.damp_attack_motion(if knockback_level > 0 { 2 } else { 1 });
        attack.is_none_or(DamageToken::is_current_life)
    }
}
