// Last verified for Bedrock 1.26.51 / protocol 2193.

use pumpkin_macros::packet;

use crate::serial::PacketRead;

/// Sent by the client when it enables or disables an empty crafter input slot.
#[derive(Debug, PacketRead)]
#[packet(306)]
pub struct SPlayerToggleCrafterSlotRequest {
    /// Block position coordinates are three little-endian int32 values on this packet.
    pub position_x: i32,
    pub position_y: i32,
    pub position_z: i32,
    /// Crafter input slot from 0 through 8.
    pub slot_index: u8,
    /// Whether the requested slot should be disabled.
    pub is_disabled: bool,
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use pumpkin_util::math::position::BlockPos;

    use crate::{Packet, serial::PacketRead};

    use super::SPlayerToggleCrafterSlotRequest;

    #[test]
    fn toggle_crafter_slot_uses_protocol_2193_wire_layout() {
        assert_eq!(<SPlayerToggleCrafterSlotRequest as Packet>::PACKET_ID, 306);

        let mut payload = Vec::new();
        payload.extend_from_slice(&8i32.to_le_bytes());
        payload.extend_from_slice(&64i32.to_le_bytes());
        payload.extend_from_slice(&(-3i32).to_le_bytes());
        payload.extend_from_slice(&[7, 1]);

        let mut cursor = Cursor::new(payload);
        let packet = SPlayerToggleCrafterSlotRequest::read(&mut cursor).unwrap();
        assert_eq!(
            BlockPos::new(packet.position_x, packet.position_y, packet.position_z),
            BlockPos::new(8, 64, -3)
        );
        assert_eq!(packet.slot_index, 7);
        assert!(packet.is_disabled);
        assert_eq!(cursor.position(), 14);
    }
}
