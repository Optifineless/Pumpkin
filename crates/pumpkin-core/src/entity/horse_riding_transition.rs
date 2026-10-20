//! Horse-local attempt invalidation at existing riding and dimension transition owners.
use super::{Entity, passive::horse::HorseEntity};

impl Entity {
    pub(super) fn invalidate_horse_ride_attempt(&self) {
        let world = self.world.load_full();
        if self.entity_type == &pumpkin_data::entity::EntityType::HORSE
            && let Some(current) = world.get_entity_by_id(self.entity_id)
            && std::ptr::eq(current.get_entity(), self)
            && let Some(horse) = current.cast_any().downcast_ref::<HorseEntity>()
        {
            horse.invalidate_ride_attempt();
        }
        if let Some(vehicle) = self.get_vehicle()
            && let Some(horse) = vehicle.cast_any().downcast_ref::<HorseEntity>()
        {
            horse.invalidate_ride_attempt();
        }
    }
}
