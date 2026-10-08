//! TrialSpawner.tickServer without holding gameplay state across plugin callbacks.
use super::{TrialSpawner, TrialSpawnerBlockEntity};
use crate::world::World;
use std::sync::{Arc, PoisonError};

impl TrialSpawnerBlockEntity {
    pub(super) fn tick_without_guard(&self, world: &Arc<World>, is_ominous: bool) {
        self.with_tick_snapshot(|spawner, validate| {
            spawner.tick_server_validated(world, self.position, is_ominous, &|| {
                world.get_block(&self.position).id == pumpkin_data::BlockId::TRIAL_SPAWNER
                    && world.get_block_entity(&self.position).is_some_and(|block| {
                        block
                            .as_any()
                            .downcast_ref::<Self>()
                            .is_some_and(|live| std::ptr::eq(live, self))
                    })
                    && pumpkin_data::block_properties::TrialSpawnerLikeProperties::from_state_id(
                        world.get_block_state_id(&self.position),
                    )
                    .trial_spawner_state
                        == pumpkin_data::block_properties::TrialSpawnerState::Active
                    && validate()
            });
        });
    }

    // A callback can read NBT or replace the configuration. Never overwrite that replacement.
    fn with_tick_snapshot(&self, tick: impl FnOnce(&mut TrialSpawner, &dyn Fn() -> bool)) {
        let before = self
            .trial_spawner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut next = before.clone();
        tick(&mut next, &|| {
            *self
                .trial_spawner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                == before
        });
        let mut current = self
            .trial_spawner
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *current == before {
            *current = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::entities::BlockEntity;
    use pumpkin_nbt::compound::NbtCompound;
    use pumpkin_util::math::position::BlockPos;

    #[test]
    fn trial_callbacks_can_read_nbt_and_invalidate_pending_state() {
        let block = TrialSpawnerBlockEntity::from_nbt(&NbtCompound::new(), BlockPos::new(0, 64, 0));
        block.with_tick_snapshot(|pending, validate| {
            // Fail immediately rather than hang if the tick retains the mutex during callbacks.
            assert!(block.trial_spawner.try_lock().is_ok());
            block.write_nbt(&mut NbtCompound::new());
            assert!(validate());
            pending.data.total_mobs_spawned = 1;
            block.trial_spawner.lock().unwrap().data.total_mobs_spawned = 9;
            assert!(!validate());
        });
        assert_eq!(
            block.trial_spawner.lock().unwrap().data.total_mobs_spawned,
            9
        );
        block.with_tick_snapshot(|pending, validate| {
            assert!(validate());
            pending.data.total_mobs_spawned = 10;
        });
        assert_eq!(
            block.trial_spawner.lock().unwrap().data.total_mobs_spawned,
            10
        );
    }
}
