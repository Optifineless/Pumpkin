use super::{EntityBase, LivingEntity};
use crate::{block::OnLandedUponArgs, world::World};
use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;

struct Landing {
    claim: u64,
    lifecycle: u64,
    distance: f32,
    particle_distance: f32,
    world: Arc<World>,
    position: Vector3<f64>,
}

impl Landing {
    fn is_current(&self, living: &LivingEntity, caller: &dyn EntityBase) -> bool {
        living.damage_owner.landing_is_current(self.claim)
            && living.fall_context_current(caller, self.lifecycle, &self.world, self.position)
    }

    fn can_continue(&self, living: &LivingEntity, caller: &dyn EntityBase) -> bool {
        let _owner = living.own_damage();
        self.is_current(living, caller) && living.fall_distance.load() == self.distance
    }
}

impl LivingEntity {
    pub(super) fn fall_context_current(
        &self,
        caller: &dyn EntityBase,
        lifecycle: u64,
        world: &Arc<World>,
        position: Vector3<f64>,
    ) -> bool {
        self.damage_lifecycle() == lifecycle
            && self.entity.pos.load() == position
            && self.fall_session_current(caller, world, false)
    }

    pub(super) fn fall_session_current(
        &self,
        caller: &dyn EntityBase,
        world: &World,
        ender_pearl_transfer: bool,
    ) -> bool {
        std::ptr::eq(self.entity.world.load_full().as_ref(), world)
            && !self.entity.is_removed()
            && caller.get_player().is_none_or(|player| {
                (player.has_client_loaded() || ender_pearl_transfer)
                    && !player.client.closed()
                    && world
                        .get_player_by_uuid(player.gameprofile.id)
                        .is_some_and(|current| std::ptr::eq(player, current.as_ref()))
            })
    }

    /// Accounts accepted descent before dispatching the supporting block's landing callback.
    pub fn fall(
        &self,
        caller: &dyn EntityBase,
        height_difference: f64,
        ground: bool,
        dont_damage: bool,
    ) {
        if caller
            .get_mob()
            .is_some_and(super::super::mob::Mob::check_fall_damage)
        {
            return;
        }
        let landing = {
            let _owner = self.own_damage();
            let world = self.entity.world.load_full();
            if !self.fall_session_current(caller, &world, false) {
                return;
            }
            let particle_distance = self.fall_distance.load();
            // Entity.checkFallDamage includes the final descending packet before onGround.
            if height_difference < 0.0 {
                if !self.should_prevent_fall_damage() && !self.should_prevent_fall_damage_in_area()
                {
                    let distance = self
                        .damage_owner
                        .fall_distance_before_descent(self.fall_distance.load())
                        - height_difference as f32;
                    self.fall_distance.store(distance);
                } else if !ground {
                    self.fall_distance.store(0.0);
                }
            }
            if ground && self.fall_distance.load() > 0.0 {
                // TODO(#231 B3): Hand off a distinct landing completed during an active callback.
                self.damage_owner
                    .claim_landing(self.fall_distance.load())
                    .map(|claim| Landing {
                        claim,
                        lifecycle: self.damage_lifecycle(),
                        distance: self.fall_distance.load(),
                        particle_distance,
                        world: self.entity.world.load_full(),
                        position: self.entity.pos.load(),
                    })
            } else {
                if ground {
                    self.fall_distance.store(0.0);
                }
                None
            }
        };
        if let Some(landing) = landing {
            self.dispatch_landing(caller, &landing, dont_damage);
            let _owner = self.own_damage();
            if self.damage_owner.landing_is_current(landing.claim)
                && self.damage_lifecycle() == landing.lifecycle
                && self.fall_session_current(caller, &landing.world, false)
                && !self.damage_owner.landing_has_descent()
                && self.fall_distance.load() == landing.distance
            {
                // Entity.checkFallDamage resets after the callback, in the admitted life.
                self.fall_distance.store(0.0);
            }
            self.damage_owner.release_landing(landing.claim);
        } else if !ground && height_difference < 0.0 {
            crate::block::blocks::honey::HoneyBlock::reset_player_fall_distance(
                caller,
                height_difference,
            );
        }
    }

    fn dispatch_landing(&self, caller: &dyn EntityBase, landing: &Landing, dont_damage: bool) {
        if !landing.can_continue(self, caller) {
            return;
        }
        if let Some(player) = caller.get_player() {
            // ServerPlayer.checkFallDamage emits particles before Entity accumulates descent.
            player.check_mace_landing_particles(landing.particle_distance);
        }
        if !landing.can_continue(self, caller) {
            return;
        }
        self.on_changed_block(caller, self.entity.block_pos.load());
        if !landing.can_continue(self, caller)
            || dont_damage
            || self.should_prevent_fall_damage()
            || self.should_prevent_fall_damage_in_area()
            || self.is_immune_to_fall_damage()
        {
            return;
        }
        let position = self.entity.get_pos_with_y_offset(0.2).0;
        let block = landing.world.get_block(&position);
        if let Some(behavior) = landing.world.block_registry.get_pumpkin_block(block.id) {
            behavior.on_landed_upon(OnLandedUponArgs {
                world: &landing.world,
                position: &position,
                fall_distance: landing.distance,
                entity: caller,
            });
        } else {
            self.handle_fall_damage(caller, landing.distance, 1.0);
        }
    }
}
