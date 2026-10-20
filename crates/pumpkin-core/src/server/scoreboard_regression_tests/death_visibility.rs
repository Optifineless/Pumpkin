use super::*;
use crate::{
    entity::{EntityBase, player::Player},
    net::{
        ClientPlatform, GameProfile, PlayerConfig,
        java::combat_test_support::{OutgoingPacket, captured_client},
    },
};
use arc_swap::ArcSwap;
use tokio::sync::mpsc::UnboundedReceiver;

fn player(world: &Arc<World>, name: &str) -> (Arc<Player>, UnboundedReceiver<OutgoingPacket>) {
    let profile = GameProfile {
        id: uuid::Uuid::new_v4(),
        name: name.into(),
        properties: ArcSwap::from_pointee(Vec::new()),
        profile_actions: None,
    };
    let (client, packets) = captured_client(profile.clone());
    let player = Arc::new(Player::new(
        Arc::new(ClientPlatform::Java(client)),
        profile,
        PlayerConfig::default(),
        world,
        pumpkin_util::GameMode::Survival,
    ));
    player.set_client_loaded(true);
    world.players.rcu(|players| {
        let mut players = (**players).clone();
        players.push(player.clone());
        players
    });
    (player, packets)
}

fn packets(receiver: &mut UnboundedReceiver<OutgoingPacket>) -> Vec<bytes::Bytes> {
    let mut packets = Vec::new();
    while let Ok(packet) = receiver.try_recv() {
        lifecycle::capture(packet, &mut packets);
    }
    packets
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_death_visibility_routes_across_dimensions() {
    for (visibility, expected) in [
        ("always", [1, 1, 1, 1]),
        ("never", [0, 0, 0, 0]),
        ("hideForOwnTeam", [0, 0, 1, 1]),
        ("hideForOtherTeams", [0, 1, 0, 0]),
    ] {
        let directory = tempfile::tempdir().unwrap();
        with_worlds(directory.path(), None, |server, worlds| async move {
            let (victim, victim_packets) = player(&worlds[0], "Victim");
            let (_own, own_packets) = player(&worlds[1], "OwnTeam");
            let (_other, other_packets) = player(&worlds[2], "OtherTeam");
            let (_neutral, neutral_packets) = player(&worlds[2], "Neutral");
            let mut receivers = [victim_packets, own_packets, other_packets, neutral_packets];
            for input in [
                "team add red",
                "team add blue",
                "team join red Victim",
                "team join red OwnTeam",
                "team join blue OtherTeam",
            ] {
                command(&server, &worlds[0], input);
            }
            if visibility != "always" {
                command(
                    &server,
                    &worlds[0],
                    &format!("team modify red deathMessageVisibility {visibility}"),
                );
            }

            // Persist the rule and reinstall the actual decoded saved owner before the death.
            save(&server).await;
            let saved = read_scoreboard(&server.basic_config.get_world_path());
            *server.scoreboard.lock().unwrap() =
                crate::world::scoreboard::Scoreboard::from_save_data(payload(&saved)).unwrap();
            for receiver in &mut receivers {
                packets(receiver);
            }
            assert!(victim.damage(
                victim.as_ref(),
                f32::MAX,
                pumpkin_data::damage::DamageType::GENERIC_KILL
            ));
            assert_eq!(victim.get_living_entity().unwrap().health.load(), 0.0);
            for (index, receiver) in receivers.iter_mut().enumerate() {
                let received = packets(receiver);
                assert_eq!(
                    packet_count(
                        &received,
                        pumpkin_data::packet::clientbound::play::SYSTEM_CHAT.0
                    ),
                    expected[index],
                    "{visibility}, recipient {index}"
                );
                if index == 0 {
                    assert_eq!(
                        packet_count(
                            &received,
                            pumpkin_data::packet::clientbound::play::PLAYER_COMBAT_KILL.0
                        ),
                        1,
                        "Victim death screen must survive {visibility}"
                    );
                }
            }
        })
        .await;
    }
}
