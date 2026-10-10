use super::{EntityBase, living::LivingEntity};
use pumpkin_data::attributes::Attributes;
use std::sync::atomic::Ordering;

/// Applies living fire-duration scaling and the cancellable combustion event.
pub fn ignite_for_ticks<T: EntityBase + ?Sized>(target: &T, ticks: u32) {
    let entity = target.get_entity();
    let ticks = if target.get_player().is_some_and(|player| {
        player
            .abilities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .invulnerable
    }) {
        // Player.setRemainingFireTicks clamps ability-invulnerable players to one tick.
        1
    } else {
        target
            .get_living_entity()
            .map_or(ticks, |living| living.scaled_ignition_ticks(ticks))
    };
    let mut event = crate::plugin::api::events::entity::entity_combust::EntityCombustEvent::new(
        entity.entity_id,
        ticks as f32 / 20.0,
    );
    if let Some(server) = entity.world.load().server.upgrade() {
        server.plugin_manager.fire_blocking(&server, &mut event);
    }
    if !event.cancelled {
        entity.fire_ticks.fetch_max(ticks as i32, Ordering::Relaxed);
    }
}

impl LivingEntity {
    // LivingEntity.igniteForTicks rounds after multiplying the effective BURNING_TIME.
    pub(crate) fn scaled_ignition_ticks(&self, ticks: u32) -> u32 {
        ignition_ticks(ticks, self.get_attribute_value(&Attributes::BURNING_TIME))
    }
}

fn ignition_ticks(ticks: u32, burning_time: f64) -> u32 {
    (f64::from(ticks) * burning_time).ceil() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn melee_ignition_rounds_after_burning_time_scaling() {
        assert_eq!(ignition_ticks(81, 0.5), 41);
        assert_eq!(ignition_ticks(80, 1.25), 100);
    }
}
