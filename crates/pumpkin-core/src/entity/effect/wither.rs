use pumpkin_data::damage::DamageType;

use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;

pub struct WitherMobEffect;

impl MobEffect for WitherMobEffect {
    fn should_apply_effect_tick(&self, duration: i32, amplifier: u8) -> bool {
        // WitherMobEffect.shouldApplyEffectTickThisTick uses Java's masked shift.
        let interval = 40i32.wrapping_shr(u32::from(amplifier));
        interval <= 0 || duration % interval == 0
    }

    fn apply_effect_tick(&self, living: &LivingEntity, _amplifier: u8) -> bool {
        let dyn_self = living
            .entity
            .world
            .load()
            .get_entity_by_id(living.entity.entity_id);
        if let Some(dyn_self) = dyn_self {
            dyn_self.damage(&*dyn_self, 1.0, DamageType::WITHER);
        }
        true
    }
}
