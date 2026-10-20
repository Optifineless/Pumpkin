//! Logical arrival reservations make named-world retirement refuse an in-flight player.

use std::sync::{Arc, PoisonError};

use super::World;

#[cfg(test)]
#[path = "player_arrival_tests.rs"]
mod tests;

// No lock or shutdown-blocking permit survives admission. Named unload returns an
// occupied-world error, including when called from an arrival packet callback.
pub struct PlayerArrival {
    world: Arc<World>,
    uuid: uuid::Uuid,
}

impl Drop for PlayerArrival {
    fn drop(&mut self) {
        self.world
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .player_arrivals
            .remove(&self.uuid);
    }
}

impl World {
    pub(crate) fn reserve_player_arrival(
        self: &Arc<Self>,
        uuid: uuid::Uuid,
    ) -> Option<PlayerArrival> {
        let server = self.server.upgrade()?;
        if !server
            .worlds
            .load()
            .iter()
            .any(|world| Arc::ptr_eq(world, self))
        {
            return None;
        }
        let mut state = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.stop_admission
            || self.level.cancel_token.is_cancelled()
            || !state.player_arrivals.insert(uuid)
        {
            return None;
        }
        Some(PlayerArrival {
            world: self.clone(),
            uuid,
        })
    }

    pub(crate) async fn begin_named_world_unload(&self) -> Result<(), usize> {
        loop {
            {
                let mut state = self
                    .pending_block_entity_loads
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let players = self.players.load().len() + state.player_arrivals.len();
                if players != 0 {
                    return Err(players);
                }
                // Preserve nested leaf admission until existing synchronous mutations drain.
                if state.active_mutations.is_empty() {
                    state.stop_admission = true;
                    return Ok(());
                }
            }
            tokio::task::yield_now().await;
        }
    }
}
