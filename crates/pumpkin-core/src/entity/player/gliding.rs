use super::Player;
use crate::{
    entity::{Entity, EntityBase, Flag},
    plugin::api::events::entity::entity_toggle_glide::EntityToggleGlideEvent,
    world::World,
};
use std::sync::{Arc, atomic::Ordering::Relaxed};

impl Player {
    // Player.tryToStartFallFlying / LivingEntity.stopFallFlying, shared by both editions.
    pub(crate) fn request_gliding(&self, gliding: bool) {
        let (world, life) = {
            let _owner = self.living_entity.own_damage();
            let world = self.world();
            let life = self.living_entity.damage_lifecycle();
            if !self.glide_request_current(&world, life) || !self.glide_transition_allowed(gliding)
            {
                return;
            }
            (world, life)
        };
        let Some(server) = world.server.upgrade() else {
            return;
        };
        let mut event = EntityToggleGlideEvent::new(self.entity_id(), gliding);
        server.plugin_manager.fire_blocking(&server, &mut event);
        let owner = self.living_entity.own_damage();
        if event.cancelled
            || event.is_gliding != gliding
            || !self.glide_request_current(&world, life)
            || !self.glide_transition_allowed(gliding)
        {
            return;
        }
        self.get_entity().set_fall_flying_state(gliding);
        drop(owner);
        self.get_entity().send_bedrock_flags();
    }

    fn glide_request_current(&self, world: &Arc<World>, life: u64) -> bool {
        self.living_entity.damage_lifecycle() == life
            && Arc::ptr_eq(&self.world(), world)
            && world.contains_player_instance(self)
            && self.has_client_loaded()
            && !self.client.closed()
            && !self.get_entity().is_removed()
            && !self.living_entity.dead.load(Relaxed)
            && self.living_entity.health.load() > 0.0
    }

    fn glide_transition_allowed(&self, gliding: bool) -> bool {
        if gliding {
            self.living_entity.can_start_fall_flying(self)
        } else {
            self.get_entity().is_fall_flying()
        }
    }
}

impl Entity {
    // Keep the transition atomic with request admission; publish only after releasing ownership.
    pub(in crate::entity) fn set_fall_flying_state(&self, fall_flying: bool) {
        assert_ne!(self.fall_flying.load(Relaxed), fall_flying);
        self.fall_flying.store(fall_flying, Relaxed);
        self.fall_fly_ticks.store(0, Relaxed);
        self.set_flag_state(Flag::FallFlying, fall_flying);
    }
}
