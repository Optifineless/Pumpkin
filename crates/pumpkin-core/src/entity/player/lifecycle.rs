use super::Player;
use crate::entity::{EntityBase, RemovalReason};

impl Player {
    /// PlayerList.remove retires this connection's player before its logout snapshot is captured.
    pub(crate) fn mark_departing(&self) {
        if let Some(vehicle) = self.get_entity().get_vehicle() {
            self.root_vehicle_uuid
                .store(Some(vehicle.get_entity().entity_uuid));
        }
        self.get_entity()
            .mark_removed_only(RemovalReason::UnloadedWithPlayer);
    }

    /// ServerPlayer.setGameMode calls Entity.stopRiding when a player becomes a spectator.
    pub(crate) fn stop_riding_for_spectator(&self) {
        if let Some(vehicle) = self.get_entity().get_vehicle() {
            vehicle.get_entity().remove_passenger(self.entity_id());
        }
    }
}
