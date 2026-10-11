use std::sync::{Arc, Mutex};

use super::World;

#[derive(Default)]
pub struct TransferAdmission {
    state: Mutex<AdmissionState>,
    #[cfg(test)]
    unload_gate: Mutex<Option<UnloadGate>>,
}

#[cfg(test)]
type UnloadGate = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);

#[derive(Default)]
struct AdmissionState {
    transfers: usize,
    unloading: bool,
}

pub struct WorldAdmission(Arc<World>);

pub struct WorldUnloading<'a>(&'a World);

impl Drop for WorldUnloading<'_> {
    fn drop(&mut self) {
        // ServerPlayer.teleport uses a live ServerLevel; cancelling before shutdown keeps it live.
        if !self.0.level.cancel_token.is_cancelled() {
            self.0
                .transfer_admission
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .unloading = false;
        }
    }
}

impl Drop for WorldAdmission {
    fn drop(&mut self) {
        self.0
            .transfer_admission
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .transfers -= 1;
    }
}

impl World {
    /// Reserves a registered, running destination until transfer publication completes.
    pub(crate) fn admit_transfer(self: &Arc<Self>) -> Option<WorldAdmission> {
        let mut state = self
            .transfer_admission
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let server = self.server.upgrade()?;
        if state.unloading
            || self.level.cancel_token.is_cancelled()
            || !server
                .worlds
                .load()
                .iter()
                .any(|world| Arc::ptr_eq(world, self))
        {
            return None;
        }
        // Entity.teleportToPortalDestination / ServerPlayer.teleport use a live ServerLevel.
        state.transfers += 1;
        Some(WorldAdmission(self.clone()))
    }

    /// Closes admission before checking membership; no callback runs under the state lock.
    /// Keep the claim until unload finishes; cancellation before shutdown reopens admission.
    pub(crate) fn begin_unload(&self, name: &str) -> Result<WorldUnloading<'_>, String> {
        let mut state = self
            .transfer_admission
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.unloading {
            return Err("World is already unloading".to_owned());
        }
        if state.transfers != 0 {
            return Err(format!(
                "Cannot unload world '{name}': player transfers are still pending"
            ));
        }
        let player_count = self.players.load().len();
        if player_count != 0 {
            return Err(format!(
                "Cannot unload world '{name}': {player_count} players are still in this world"
            ));
        }
        state.unloading = true;
        Ok(WorldUnloading(self))
    }

    #[cfg(test)]
    pub(crate) fn gate_unload_for_test(
        &self,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let (ready, started) = tokio::sync::oneshot::channel();
        let (resume, gate) = tokio::sync::oneshot::channel();
        *self
            .transfer_admission
            .unload_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((ready, gate));
        (started, resume)
    }

    #[cfg(test)]
    pub(crate) async fn pause_unload_for_test(&self) {
        let gate = self
            .transfer_admission
            .unload_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some((ready, resume)) = gate {
            let _ = ready.send(());
            let _ = resume.await;
        }
    }
}
