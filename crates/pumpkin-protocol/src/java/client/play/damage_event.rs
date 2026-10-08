use crate::ClientPacket;
use crate::VarInt;
use crate::ser::NetworkWriteExt;
use pumpkin_data::packet::clientbound::play::DAMAGE_EVENT;
use pumpkin_macros::java_packet;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::version::JavaMinecraftVersion;

/// Notifies the client that an entity has taken damage.
///
/// This packet is used to trigger damage animations (like the red tint on mobs),
/// directional knockback visuals, and sound effects. It provides the client
/// with specific details about the damage source to ensure the visual feedback
/// matches the cause.
#[java_packet(DAMAGE_EVENT)]
pub struct CDamageEvent {
    /// The Entity ID of the entity taking damage.
    pub entity_id: VarInt,
    /// The ID of the damage type (references the `minecraft:damage_type` registry).
    /// Examples: `magic`, `fall`, `on_fire`, or `arrow`.
    pub source_type_id: VarInt,
    /// The Entity ID of the actual cause of the damage (e.g., the player who shot the arrow).
    /// Set to 0 if there is no specific entity cause.
    pub source_cause_id: VarInt,
    /// The Entity ID of the direct damager (e.g., the arrow entity itself).
    /// Set to 0 only when the direct entity is absent.
    pub source_direct_id: VarInt,
    /// The coordinates of the damage source. Used by the client to calculate
    /// the direction of the "damage tilt" camera effect.
    pub source_position: Option<Vector3<f64>>,
}

impl CDamageEvent {
    #[must_use]
    pub fn new(
        entity_id: VarInt,
        source_type_id: VarInt,
        source_cause_id: Option<VarInt>,
        source_direct_id: Option<VarInt>,
        source_position: Option<Vector3<f64>>,
    ) -> Self {
        Self {
            entity_id,
            source_type_id,
            source_cause_id: source_cause_id.map_or(VarInt(0), |id| VarInt(id.0 + 1)),
            source_direct_id: source_direct_id.map_or(VarInt(0), |id| VarInt(id.0 + 1)),
            source_position,
        }
    }
}

impl ClientPacket for CDamageEvent {
    fn write_packet_data(
        &self,
        mut write: impl std::io::Write,
        _version: &JavaMinecraftVersion,
    ) -> Result<(), crate::ser::WritingError> {
        write.write_var_int(&self.entity_id)?;
        write.write_var_int(&self.source_type_id)?;
        write.write_var_int(&self.source_cause_id)?;
        write.write_var_int(&self.source_direct_id)?;
        if let Some(pos) = &self.source_position {
            write.write_bool(true)?;
            write.write_f64(pos.x)?;
            write.write_f64(pos.y)?;
            write.write_f64(pos.z)?;
        } else {
            write.write_bool(false)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_damage_packet_keeps_distinct_victim_owner_and_projectile_ids() {
        // ClientboundDamageEventPacket.write: victim, type, cause+1, direct+1, absent raw position.
        let packet = CDamageEvent::new(
            VarInt(300),
            VarInt(7),
            Some(VarInt(128)),
            Some(VarInt(42)),
            None,
        );
        let mut bytes = Vec::new();
        packet
            .write_packet_data(&mut bytes, &JavaMinecraftVersion::V_26_3)
            .unwrap();
        assert_eq!(bytes, [0xac, 0x02, 7, 0x81, 0x01, 43, 0]);
    }
}
