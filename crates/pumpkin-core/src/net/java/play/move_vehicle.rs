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
        let driver = entity.begin_movement_emission(player.as_ref());
        let emission = vehicle_entity.begin_movement_emission(vehicle.as_ref());
        let vehicle_start = vehicle_entity.pos.load();
        let movement = pos - vehicle_start;
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
        let controlling = vehicle_entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .first()
            .cloned();
        if driver.is_some_and(|driver| driver.is_current(player.as_ref()))
            && controlling.is_some_and(|current| std::ptr::eq(current.get_entity(), entity))
            && entity
                .get_vehicle()
                .is_some_and(|current| Arc::ptr_eq(&current, &vehicle))
            && let Some(emission) = emission
        {
            let support =
                vehicle_entity.find_supporting_block(vehicle.as_ref(), packet.on_ground, movement);
            emission.with_support(support).finish(
                vehicle.as_ref(),
                vehicle_start,
                pos,
                movement,
                packet.on_ground,
            );
        }
    }
}
