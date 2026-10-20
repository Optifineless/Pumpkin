use super::{
    lifecycle::{capture, collect},
    *,
};
use crate::entity::player::Player;
use crate::net::{
    ClientPlatform, GameProfile, PlayerConfig,
    java::combat_test_support::{OutgoingPacket, captured_client},
};
use arc_swap::ArcSwap;
use pumpkin_util::math::vector2::Vector2;
use tokio::sync::mpsc::UnboundedReceiver;

pub(super) async fn join_player(
    server: &Arc<Server>,
    worlds: &[Arc<World>],
) -> (Arc<Player>, UnboundedReceiver<OutgoingPacket>) {
    let profile = GameProfile {
        id: uuid::Uuid::new_v4(),
        name: "scoreboard-join".into(),
        properties: ArcSwap::from_pointee(Vec::new()),
        profile_actions: None,
    };
    let (client, mut receive) = captured_client(profile.clone());
    let (joined, early_packets) = collect(
        server.add_player(
            Arc::new(ClientPlatform::Java(client)),
            profile,
            Some(PlayerConfig::default()),
        ),
        &mut receive,
    )
    .await;
    let (player, world) = joined.unwrap();
    assert_eq!(
        packet_count(
            &early_packets,
            pumpkin_data::packet::clientbound::play::SET_PLAYER_TEAM.0
        ),
        0
    );
    command(server, &worlds[1], "team modify red color blue");
    let mut early_packets = Vec::new();
    while let Ok(packet) = receive.try_recv() {
        capture(packet, &mut early_packets);
    }
    assert_eq!(
        packet_count(
            &early_packets,
            pumpkin_data::packet::clientbound::play::SET_PLAYER_TEAM.0
        ),
        0,
        "A pre-snapshot joining player received a premature team update"
    );
    player.client.java().unwrap().set_player(player.clone());
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    let ((), packets) = collect(
        world.spawn_java_player(&server.basic_config, &player, server),
        &mut receive,
    )
    .await;
    for id in [
        pumpkin_data::packet::clientbound::play::SCOREBOARD_OBJECTIVE.0,
        pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0,
        pumpkin_data::packet::clientbound::play::SET_PLAYER_TEAM.0,
    ] {
        assert_eq!(
            packet_count(&packets, id),
            1,
            "Login omitted or repeated shared scoreboard packet {id}"
        );
    }
    (player, receive)
}
