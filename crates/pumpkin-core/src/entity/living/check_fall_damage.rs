use super::LivingEntity;
use crate::{block::OnLandedUponArgs, entity::EntityBase};
use std::cell::Cell;

thread_local! {
    static FALL_CALLBACK_LIFE: Cell<Option<(i32, u64)>> = const { Cell::new(None) };
}

struct FallCallbackLife(Option<(i32, u64)>);

impl FallCallbackLife {
    fn enter(living: &LivingEntity) -> Self {
        Self(FALL_CALLBACK_LIFE.with(|current| {
            current.replace(Some((living.entity.entity_id, living.damage_lifecycle())))
        }))
    }
}

impl Drop for FallCallbackLife {
    fn drop(&mut self) {
        FALL_CALLBACK_LIFE.with(|current| current.set(self.0));
    }
}

impl LivingEntity {
    // PlayerList.respawn replaces the object; an old Block.fallOn cannot hurt its replacement.
    pub(super) fn fall_callback_life_is_current(&self) -> bool {
        FALL_CALLBACK_LIFE.with(|current| {
            current.get().is_none_or(|(entity_id, lifecycle)| {
                entity_id != self.entity.entity_id || lifecycle == self.damage_lifecycle()
            })
        })
    }

    // LivingEntity.checkFallDamage -> Entity.checkFallDamage resets fall distance after Block.fallOn.
    pub fn fall(
        &self,
        caller: &dyn EntityBase,
        height_difference: f64,
        ground: bool,
        dont_damage: bool,
    ) {
        let owner = self.own_damage();
        if self.is_respawning() {
            return;
        }
        if caller
            .get_mob()
            .is_some_and(crate::entity::mob::Mob::check_fall_damage)
        {
            return;
        }
        if ground {
            let fall_distance = self.fall_distance.load();
            if let Some(player) = caller.get_player() {
                player.check_mace_landing_particles(fall_distance);
            }
            if fall_distance > 0.0 {
                {
                    let _released = super::damage_transaction::suspend_damage();
                    self.on_changed_block(caller, self.entity.block_pos.load());
                };
                if !owner.is_current_life() || self.is_respawning() {
                    return;
                }
            }
            if fall_distance <= 0.0
                || dont_damage
                || self.should_prevent_fall_damage()
                || self.should_prevent_fall_damage_in_area()
                || self.is_immune_to_fall_damage()
            {
                self.fall_distance.store(0.0);
                return;
            }
            let world = self.entity.world.load();
            let landing_pos = self.entity.get_pos_with_y_offset(0.2).0;
            let block = world.get_block(&landing_pos);
            let pumpkin_block = world.block_registry.get_pumpkin_block(block.id);
            let _life = FallCallbackLife::enter(self);
            {
                // Block.fallOn may run a block update cascade; retain no combat owner there.
                let _released = super::damage_transaction::suspend_damage();
                if let Some(pumpkin_block) = pumpkin_block {
                    pumpkin_block.on_landed_upon(OnLandedUponArgs {
                        world: &world,
                        position: &landing_pos,
                        fall_distance,
                        entity: caller,
                    });
                } else {
                    self.handle_fall_damage(caller, fall_distance, 1.0);
                }
            }
            if !owner.is_current_life() || self.is_respawning() {
                return;
            }
            // Entity.checkFallDamage resets only after the landing callback records damage.
            self.fall_distance.store(0.0);
        } else if height_difference < 0.0 {
            let new_fall_distance = if !self.should_prevent_fall_damage()
                && !self.should_prevent_fall_damage_in_area()
            {
                let distance = self.fall_distance.load();
                distance - (height_difference as f32)
            } else {
                0f32
            };
            self.fall_distance.store(new_fall_distance);
            crate::block::blocks::honey::HoneyBlock::reset_player_fall_distance(
                caller,
                height_difference,
            );
        }
    }
}
