// Last verified for v2169

use crate::{
    bedrock::network_item::NetworkItemStackDescriptor, codec::var_ulong::VarULong,
    serial::PacketWrite,
};
use pumpkin_macros::packet;

/// Sends the complete armor state for an entity to Bedrock clients.
#[derive(PacketWrite, Debug)]
#[packet(32)]
pub struct CMobArmorEquipment {
    /// Runtime entity ID of the equipped actor.
    pub target_runtime_id: VarULong,
    /// Item equipped in the head slot.
    pub head: NetworkItemStackDescriptor,
    /// Item equipped in the chest slot.
    pub torso: NetworkItemStackDescriptor,
    /// Item equipped in the legs slot.
    pub legs: NetworkItemStackDescriptor,
    /// Item equipped in the feet slot.
    pub feet: NetworkItemStackDescriptor,
    /// Item equipped in the body slot.
    pub body: NetworkItemStackDescriptor,
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "Serializing the fixed packet fixture must succeed"
)]
mod tests {
    use super::*;
    use crate::{Packet, serial::PacketWrite};

    #[test]
    fn mob_armor_equipment_uses_packet_id_and_vanilla_slot_order() {
        let packet = CMobArmorEquipment {
            target_runtime_id: VarULong(42),
            head: NetworkItemStackDescriptor {
                id: 1,
                ..NetworkItemStackDescriptor::default()
            },
            torso: NetworkItemStackDescriptor {
                id: 2,
                ..NetworkItemStackDescriptor::default()
            },
            legs: NetworkItemStackDescriptor {
                id: 3,
                ..NetworkItemStackDescriptor::default()
            },
            feet: NetworkItemStackDescriptor {
                id: 4,
                ..NetworkItemStackDescriptor::default()
            },
            body: NetworkItemStackDescriptor {
                id: 5,
                ..NetworkItemStackDescriptor::default()
            },
        };

        assert_eq!(<CMobArmorEquipment as Packet>::PACKET_ID, 32);
        let mut encoded = Vec::new();
        packet.write(&mut encoded).unwrap();

        let mut expected = vec![42];
        for item_id in 1i16..=5 {
            expected.extend_from_slice(&item_id.to_le_bytes());
            expected.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        }
        assert_eq!(encoded, expected);
    }
}
