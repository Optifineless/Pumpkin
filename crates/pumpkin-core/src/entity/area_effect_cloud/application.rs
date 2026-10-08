use super::AreaEffectCloudEntity;
use crate::entity::{
    EntityBase,
    projectile::potion_effects::{self, EffectEntry},
};
use pumpkin_data::data_component_impl::PotionDurationScaleImpl;
use std::sync::Arc;

impl AreaEffectCloudEntity {
    pub(super) fn apply_to_entities(&self) {
        let effects = self
            .effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if effects.is_empty() {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .victims
                .clear();
            return;
        }
        let candidates = self.application_candidates(&effects);
        let duration_scale = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_data_component::<PotionDurationScaleImpl>()
            .map_or(1.0, |scale| scale.scale);
        let owner = self.owner();
        for target in candidates {
            let delta = target.get_entity().pos.load() - self.entity.pos.load();
            let radius = self.radius();
            if delta.x * delta.x + delta.z * delta.z > f64::from(radius * radius) {
                continue;
            }
            {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let next_application = state.age + state.reapplication_delay;
                state
                    .victims
                    .insert(target.get_entity().entity_uuid, next_application);
            };
            // No distance falloff, refresh existing effects, and instant potency is always 0.5.
            for effect in &effects {
                potion_effects::apply_effect(
                    target.as_ref(),
                    *effect,
                    f64::from(duration_scale),
                    0.5,
                    self,
                    owner.as_deref(),
                    false,
                );
            }
            let (alive, radius) = {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let alive = state.on_use();
                (alive, state.radius)
            };
            if !alive {
                self.entity.remove();
                return;
            }
            self.sync_radius(radius);
        }
    }

    // AreaEffectCloud.serverTick admits each victim only after its reapplication delay.
    fn application_candidates(&self, effects: &[EffectEntry]) -> Vec<Arc<dyn EntityBase>> {
        let world = self.entity.world.load();
        let mut candidates: Vec<_> = world
            .get_all_at_box(&self.entity.bounding_box.load())
            .into_iter()
            .filter(|target| {
                potion_effects::affected_by_potions(target.as_ref())
                    && effects.iter().any(|effect| {
                        crate::entity::living::effects::can_be_affected(
                            target.get_entity().entity_type,
                            effect.0,
                        )
                    })
            })
            .collect();
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let age = state.age;
            state.victims.retain(|_, until| age < *until);
            candidates
                .retain(|target| !state.victims.contains_key(&target.get_entity().entity_uuid));
        };
        if let Some(server) = world.server.upgrade() {
            let mut event = crate::plugin::api::events::entity::area_effect_cloud_apply::AreaEffectCloudApplyEvent::new(
                self.entity.entity_id, candidates.iter().map(|target| target.get_entity().entity_id).collect());
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return Vec::new();
            }
            candidates.retain(|target| {
                event
                    .affected_entities
                    .contains(&target.get_entity().entity_id)
            });
        }
        candidates
    }
}
