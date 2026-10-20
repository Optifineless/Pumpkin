use super::{EntityBase, LivingEntity};
use crate::{block::OnLandedUponArgs, world::World};
use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;

/// Identifies the life, session, world and position that admitted a fall.
#[derive(Clone)]
pub struct FallContext {
    pub(super) lifecycle: u64,
    world: Arc<World>,
    position: Vector3<f64>,
}

impl FallContext {
    pub(crate) fn capture(living: &LivingEntity) -> Self {
        Self {
            lifecycle: living.damage_lifecycle(),
            world: living.entity.world.load_full(),
            position: living.entity.pos.load(),
        }
    }

    pub(crate) fn is_current(&self, living: &LivingEntity, caller: &dyn EntityBase) -> bool {
        living.fall_context_current(caller, self.lifecycle, &self.world, self.position)
    }
}

struct Landing {
    claim: u64,
    context: FallContext,
    distance: f32,
    particle_distance: f32,
}

impl Landing {
    fn is_current(&self, living: &LivingEntity, caller: &dyn EntityBase) -> bool {
        living.damage_owner.landing_is_current(self.claim)
            && self.context.is_current(living, caller)
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
                        context: FallContext::capture(self),
                        distance: self.fall_distance.load(),
                        particle_distance,
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
                && self.damage_lifecycle() == landing.context.lifecycle
                && self.fall_session_current(caller, &landing.context.world, false)
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
        let _released = super::damage_transaction::suspend_damage();
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
        let block = landing.context.world.get_block(&position);
        if let Some(behavior) = landing
            .context
            .world
            .block_registry
            .get_pumpkin_block(block.id)
        {
            behavior.on_landed_upon(OnLandedUponArgs {
                world: &landing.context.world,
                position: &position,
                fall_distance: landing.distance,
                entity: caller,
                expected_landing: Some(&landing.context),
            });
        } else {
            self.handle_landing_fall_damage_from(
                caller,
                landing.distance,
                1.0,
                pumpkin_data::damage::DamageType::FALL,
                Some(&landing.context),
            );
        }
    }
}
