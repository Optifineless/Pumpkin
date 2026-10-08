use pumpkin_data::damage::DamageType;

use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;

pub struct PoisonMobEffect;

impl MobEffect for PoisonMobEffect {
    fn should_apply_effect_tick(&self, duration: i32, amplifier: u8) -> bool {
        // PoisonMobEffect.shouldApplyEffectTickThisTick uses Java's masked shift.
        let interval = 25i32.wrapping_shr(u32::from(amplifier));
        interval <= 0 || duration % interval == 0
    }

    fn apply_effect_tick(&self, living: &LivingEntity, _amplifier: u8) -> bool {
        let current_health = living.health.load();
        if current_health > 1.0
            && let Some(dyn_self) = living
                .entity
                .world
                .load()
                .get_entity_by_id(living.entity.entity_id)
        {
            let damage_amount = (current_health - 1.0).min(1.0);
            if damage_amount > 0.0 {
                dyn_self.damage(&*dyn_self, damage_amount, DamageType::MAGIC);
            }
        }
        true
    }
}
