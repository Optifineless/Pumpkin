use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;

pub struct RegenerationMobEffect;

impl MobEffect for RegenerationMobEffect {
    fn should_apply_effect_tick(&self, duration: i32, amplifier: u8) -> bool {
        // RegenerationMobEffect.shouldApplyEffectTickThisTick uses Java's masked shift.
        let interval = 50i32.wrapping_shr(u32::from(amplifier));
        interval <= 0 || duration % interval == 0
    }

    fn apply_effect_tick(&self, living: &LivingEntity, _amplifier: u8) -> bool {
        let _owner = living.own_damage();
        let current_health = living.health.load();
        let max_health = living.get_max_health();
        if current_health < max_health && current_health > 0.0 {
            living.heal(1.0);
        }
        true
    }
}
