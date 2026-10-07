#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_move_vehicle(&self, player: &Arc<Player>, packet: &SMoveVehicle) {
        let entity = player.get_entity();
        let last_pos = entity.pos.load();
        let pos = Vector3::new(packet.x, packet.y, packet.z);
        let vehicle = entity
            .vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(vehicle) = vehicle else {
            return;
        };
        if !player.has_client_loaded() || !player.controls_vehicle(vehicle.as_ref()) {
            return;
        }
        if !pos.x.is_finite()
            || !pos.y.is_finite()
            || !pos.z.is_finite()
            || !packet.yaw.is_finite()
            || !packet.pitch.is_finite()
        {
            self.try_kick(&TextComponent::text("Invalid vehicle movement"));
            return;
        }
        let vehicle_entity = vehicle.get_entity();
        let movement = pos - vehicle_entity.pos.load();
        vehicle_entity.set_pos(pos);
        vehicle_entity.set_rotation(packet.yaw, packet.pitch);
        player.known_movement.record(movement);
        entity.set_pos(pos);
        let distance = last_pos.squared_distance_to_vec(&pos).sqrt();
        let cm = (distance * 100.0).round() as i32;
        if cm > 0 {
            let stat = player.get_movement_statistic();
            player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::Custom,
                stat as i32,
                cm,
            );
        }
        chunker::update_position(player);
    }
}
