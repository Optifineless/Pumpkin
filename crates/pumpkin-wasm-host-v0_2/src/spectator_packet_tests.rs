use crate::{
    generated_packets::deserialize_java_serverbound_packet,
    pumpkin::plugin::java_packets::ServerboundPacket,
};
use pumpkin_protocol::{
    java::server::play::{SSpectateEntity, SSpectatorAction},
    packet::MultiVersionJavaPacket,
};
use pumpkin_util::version::JavaMinecraftVersion;

#[test]
fn spectator_action_preserves_legacy_wit_and_raw_payload() {
    let version = JavaMinecraftVersion::V_26_3;
    let id = SSpectatorAction::to_id(version);
    for raw in [vec![0], vec![43], vec![43; 16], vec![128; 17]] {
        assert!(deserialize_java_serverbound_packet(id, &raw, version).is_none());
    }
    let legacy = JavaMinecraftVersion::V_26_2;
    let raw = [0x12; 16];
    let decoded = deserialize_java_serverbound_packet(SSpectateEntity::to_id(legacy), &raw, legacy);
    assert!(matches!(
        &decoded,
        Some(ServerboundPacket::SSpectateEntity(_))
    ));
    if let Some(ServerboundPacket::SSpectateEntity(record)) = decoded {
        assert_eq!(record.target.high, 0x1212121212121212);
        assert_eq!(record.target.low, 0x1212121212121212);
    }
}
