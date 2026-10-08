use super::LivingEntity;
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_util::math::vector3::Vector3;
use std::sync::Mutex;

#[derive(Default)]
pub(super) struct ImpulseContext(Mutex<ImpulseState>);

#[derive(Default)]
struct ImpulseState {
    impact: Option<Vector3<f64>>,
    grace_ticks: i32,
}

impl ImpulseContext {
    // MaceItem.calculateImpactPosition and LivingEntity.setIgnoreFallDamageFromCurrentImpulse.
    pub(super) fn mace_impact(&self, position: Vector3<f64>) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.impact.is_none_or(|impact| impact.y > position.y) {
            state.impact = Some(position);
        }
        state.grace_ticks = state.grace_ticks.max(40);
    }

    pub(super) fn tick(&self) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.grace_ticks = (state.grace_ticks - 1).max(0);
    }

    // LivingEntity.causeFallDamage uses descent below the impulse's impact position.
    pub(super) fn effective_fall_distance(&self, distance: f32, y: f64) -> f32 {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(impact) = state.impact else {
            return distance;
        };
        let effective = distance.min((impact.y - y) as f32);
        if effective <= 0.0 || state.grace_ticks == 0 {
            *state = ImpulseState::default();
        }
        effective
    }

    pub(super) fn reset(&self) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = ImpulseState::default();
    }

    pub(super) fn write_nbt(&self, nbt: &mut NbtCompound) {
        let state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        nbt.put_int(
            "current_impulse_context_reset_grace_time",
            state.grace_ticks,
        );
        if let Some(pos) = state.impact {
            nbt.put_list(
                "current_explosion_impact_pos",
                vec![
                    NbtTag::Double(pos.x),
                    NbtTag::Double(pos.y),
                    NbtTag::Double(pos.z),
                ],
            );
        }
    }

    pub(super) fn read_nbt(&self, nbt: &NbtCompound) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.grace_ticks = nbt
            .get_int("current_impulse_context_reset_grace_time")
            .unwrap_or(0);
        state.impact = nbt
            .get_list("current_explosion_impact_pos")
            .and_then(|values| {
                Some(Vector3::new(
                    values.first()?.as_numeric_double()?,
                    values.get(1)?.as_numeric_double()?,
                    values.get(2)?.as_numeric_double()?,
                ))
            });
    }
}

impl LivingEntity {
    /// Preserves the lowest mace impact and grants vanilla's 40-tick landing grace.
    pub fn protect_mace_landing(&self) {
        self.impulse.mace_impact(self.entity.pos.load());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn melee_mace_protects_landing_above_impact_and_only_descent_below_it() {
        let impulse = ImpulseContext::default();
        impulse.mace_impact(Vector3::new(0.0, 10.0, 0.0));
        impulse.mace_impact(Vector3::new(0.0, 20.0, 0.0));
        assert_eq!(impulse.effective_fall_distance(12.0, 7.0), 3.0);
        // Grace keeps the impact after a harmless landing below it.
        assert_eq!(impulse.effective_fall_distance(15.0, 8.0), 2.0);
        assert_eq!(impulse.effective_fall_distance(15.0, 12.0), -2.0);
        assert_eq!(impulse.effective_fall_distance(5.0, 0.0), 5.0);
    }

    #[test]
    fn melee_mace_grace_expires_after_forty_ticks() {
        let impulse = ImpulseContext::default();
        impulse.mace_impact(Vector3::new(0.0, 10.0, 0.0));
        for _ in 0..40 {
            impulse.tick();
        }
        assert_eq!(impulse.effective_fall_distance(20.0, 8.0), 2.0);
        assert_eq!(impulse.effective_fall_distance(20.0, 8.0), 20.0);
    }
}
