use super::MobEffect;
use crate::entity::living::LivingEntity;

pub struct AbsorptionMobEffect;

impl MobEffect for AbsorptionMobEffect {
    // AbsorptionMobEffect.shouldApplyEffectTickThisTick / applyEffectTick.
    fn should_apply_effect_tick(&self, _duration: i32, _amplifier: u8) -> bool {
        true
    }
    fn apply_effect_tick(&self, living: &LivingEntity, _amplifier: u8) -> bool {
        living.absorption.load() > 0.0
    }
}
