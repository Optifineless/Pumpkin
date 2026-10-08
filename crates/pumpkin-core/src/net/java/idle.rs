use super::JavaClient;
use crate::{
    entity::{EntityBase, player::Player},
    server::Server,
};
use pumpkin_protocol::java::server::play::{SMoveVehicle, SPlayerInput};
use std::sync::Arc;

// ServerGamePacketListenerImpl.handlePlayerKnownMovement compares to a float literal.
const MIN_MOVEMENT_SQUARED: f64 = 1.0e-5f32 as f64;

impl JavaClient {
    pub(super) fn handle_active_player_input(
        &self,
        player: &Arc<Player>,
        input: &SPlayerInput,
        server: &Arc<Server>,
    ) {
        self.handle_player_input(player, input, server);
        // ServerGamePacketListenerImpl.handlePlayerInput resets action time only after loading.
        if player.has_client_loaded() {
            player.update_last_action_time();
        }
    }

    pub(super) fn handle_active_vehicle_movement(
        &self,
        player: &Arc<Player>,
        packet: &SMoveVehicle,
    ) {
        let vehicle = player.get_entity().get_vehicle();
        let before = vehicle
            .as_ref()
            .map(|vehicle| vehicle.get_entity().pos.load());
        self.handle_move_vehicle(player, packet);
        // ServerGamePacketListenerImpl.handlePlayerKnownMovement: actual accepted displacement.
        if let (Some(vehicle), Some(before)) = (vehicle, before)
            && before.squared_distance_to_vec(&vehicle.get_entity().pos.load())
                > MIN_MOVEMENT_SQUARED
        {
            player.update_last_action_time();
        }
    }
}
