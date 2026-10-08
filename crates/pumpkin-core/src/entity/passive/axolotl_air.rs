use super::axolotl::AxolotlEntity;
use crate::entity::EntityBase;
use pumpkin_data::damage::DamageType;
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::position::BlockPos;
use std::sync::atomic::Ordering::Relaxed;

// Axolotl.getMaxAirSupply.
pub(super) const MAX_AIR_SUPPLY: i32 = 6000;

// Axolotl.handleAirSupply: death/water/rain refill; dry damage begins at -20.
const fn next_air_supply(air: i32, alive: bool, wet: bool) -> i32 {
    if alive && !wet {
        air.wrapping_sub(1)
    } else {
        MAX_AIR_SUPPLY
    }
}

// Entity.readAdditionalSaveData -> TagValueInput.getIntOr accepts every NumericTag.
pub(super) fn read_air_supply(nbt: &NbtCompound) -> i32 {
    match nbt.get("Air") {
        Some(NbtTag::Byte(value)) => i32::from(*value),
        Some(NbtTag::Short(value)) => i32::from(*value),
        Some(NbtTag::Int(value)) => *value,
        Some(NbtTag::Long(value)) => *value as i32,
        Some(NbtTag::Float(value)) => numeric_floor(f64::from(*value)),
        Some(NbtTag::Double(value)) => numeric_floor(*value),
        _ => MAX_AIR_SUPPLY,
    }
}

// FloatTag/DoubleTag.intValue use Mth.floor, including Java's narrowing and integer wrap.
fn numeric_floor(value: f64) -> i32 {
    let truncated = value as i32;
    if value < f64::from(truncated) {
        truncated.wrapping_sub(1)
    } else {
        truncated
    }
}

impl AxolotlEntity {
    pub(super) fn handle_air_supply(&self) {
        if self.mob_entity.is_no_ai() {
            return;
        }
        let entity = self.get_entity();
        let world = entity.world.load();
        // LivingEntity.isAlive includes health during the death animation.
        let alive = entity.is_alive() && self.mob_entity.living_entity.health.load() > 0.0;
        let feet = entity.block_pos.load();
        // Entity.isInRain checks the bounding-box top as well as the feet.
        let head = BlockPos::new(
            feet.0.x,
            entity.bounding_box.load().max.y.floor() as i32,
            feet.0.z,
        );
        let wet = entity.touching_water.load(Relaxed)
            || world.is_raining_at(&feet)
            || world.is_raining_at(&head);
        let air = next_air_supply(self.air_supply.load(Relaxed), alive, wet);
        self.set_air_supply(air);
        if alive && !wet && self.air_supply.load(Relaxed) <= -20 {
            self.set_air_supply(0);
            self.damage(self, 2.0, DamageType::DRY_OUT);
        }
    }

    fn set_air_supply(&self, mut air: i32) {
        let entity = self.get_entity();
        if self.air_supply.load(Relaxed) == air {
            return;
        }
        if let Some(server) = entity.world.load().server.upgrade() {
            let mut event =
                crate::plugin::api::events::entity::entity_air_change::EntityAirChangeEvent::new(
                    entity.entity_id,
                    air,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
            air = event.amount;
        }
        self.air_supply.store(air, Relaxed);
        entity.set_synced_data(
            pumpkin_data::tracked_data::entity::DATA_AIR_SUPPLY_ID,
            VarInt(air),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drying_counts_down_and_water_or_rain_refills() {
        let mut air = MAX_AIR_SUPPLY;
        for _ in 0..6019 {
            air = next_air_supply(air, true, false);
        }
        assert_eq!(air, -19);
        assert_eq!(next_air_supply(air, true, false), -20);
        assert_eq!(next_air_supply(air, true, true), 6000);
        assert_eq!(next_air_supply(air, false, false), 6000);
    }
}
