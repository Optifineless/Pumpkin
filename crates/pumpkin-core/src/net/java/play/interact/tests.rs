#![expect(
    clippy::unwrap_used,
    reason = "Packet dispatcher regression uses a valid in-memory server fixture"
)]

use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};
use bytes::Bytes;
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_protocol::{RawPacket, java::server::play::SInteract, packet::MultiVersionJavaPacket};

#[tokio::test]
async fn baseline_regression_zero_location_vector_decodes_through_play_packet_dispatch() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    server
        .worlds
        .store(std::sync::Arc::new(vec![world.clone()]));
    let player = TestPlayer::new(&world);

    // 26.3 interact payload: entity id 123, main hand, zero location, not sneaking.
    let packet = RawPacket {
        id: SInteract::to_id(CURRENT_MC_VERSION),
        payload: Bytes::from_static(&[0x7B, 0x00, 0x00, 0x00]),
    };
    player
        .client()
        .handle_play_packet(&player.player, &server, &packet)
        .unwrap();

    server.shutdown().await;
}
