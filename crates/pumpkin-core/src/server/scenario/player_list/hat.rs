use super::*;
use crate::plugin::api::events::player::player_locale_change::PlayerLocaleChangeEvent;
use pumpkin_protocol::{
    MultiVersionJavaPacket, VarInt, java::server::play::SClientInformationPlay,
};

fn settings(skin_parts: u8) -> SClientInformationPlay<'static> {
    SClientInformationPlay {
        locale: "en_us",
        view_distance: 2,
        chat_mode: VarInt(0),
        chat_colors: true,
        skin_parts,
        main_hand: VarInt(1),
        text_filtering: false,
        server_listing: false,
    }
}

fn hats(packets: &[Bytes], subject: Uuid) -> Vec<bool> {
    packets
        .iter()
        .flat_map(infos)
        .filter(|row| row.uuid == subject)
        .filter_map(|row| row.hat)
        .collect()
}

fn assert_hat_update(observer: &Session, subject: Uuid, expected: bool) {
    let packets = observer.packets();
    assert_eq!(
        hats(&packets, subject),
        vec![expected],
        "issue #296: accepted hat change missed a tab observer"
    );
    assert!(
        added(&packets).is_empty(),
        "hat update repeated membership initialization"
    );
}

#[tokio::test]
async fn b1020b_g07_player_list_hat_tracks_actual_setting() {
    run(async {
        let scenario = Scenario::new().await;
        let subject = scenario.join(Uuid::new_v4(), "HatSubject").await;
        subject.send(&scenario.server, &settings(0x3f));
        subject.packets();

        let near = scenario.join(Uuid::new_v4(), "HatNear").await;
        let initial = near.packets();
        assert_eq!(
            hats(&initial, subject.player.gameprofile.id),
            vec![false],
            "issue #296: initial tab portrait ignores disabled hat"
        );
        assert_eq!(
            initial
                .iter()
                .flat_map(infos)
                .filter(|row| row.uuid == subject.player.gameprofile.id && row.added)
                .count(),
            1,
            "initial membership control"
        );

        let far = scenario.join(Uuid::new_v4(), "HatFar").await;
        transfer(&scenario, &far, "the_nether").await;
        for observer in [&subject, &near, &far] {
            observer.packets();
        }
        for (skin_parts, expected) in [(0x7f, true), (0x3f, false)] {
            subject.send(&scenario.server, &settings(skin_parts));
            for observer in [&subject, &near, &far] {
                assert_hat_update(observer, subject.player.gameprofile.id, expected);
            }
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020b_g07_player_list_hat_unchanged_and_session_controls() {
    run(async {
        let scenario = Scenario::new().await;
        let subject = scenario.join(Uuid::new_v4(), "HatControl").await;
        let observer = scenario.join(Uuid::new_v4(), "HatWitness").await;
        subject.packets();
        observer.packets();

        // Changing a different skin layer and repeating an identical packet need no hat action.
        for skin_parts in [0x7f, 0x7e, 0x7e] {
            subject.send(&scenario.server, &settings(skin_parts));
            for session in [&subject, &observer] {
                assert!(hats(&session.packets(), subject.player.gameprofile.id).is_empty());
            }
        }

        // Keep the admitted session ready while its entity is no longer a publication source.
        subject
            .player
            .get_entity()
            .removal_reason
            .store(Some(crate::entity::RemovalReason::Discarded));
        subject.send(&scenario.server, &settings(0x3f));
        assert!(hats(&observer.packets(), subject.player.gameprofile.id).is_empty());
        subject.player.get_entity().removal_reason.store(None);

        let mut payload = Vec::new();
        settings(0x3f)
            .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
            .unwrap();
        // Route through the dispatcher with another connection's player instance.
        observer
            .client()
            .handle_play_packet(
                &subject.player,
                &scenario.server,
                &pumpkin_protocol::RawPacket {
                    id: SClientInformationPlay::to_id(CURRENT_MC_VERSION),
                    payload: payload.clone().into(),
                },
            )
            .unwrap();
        assert!(hats(&observer.packets(), subject.player.gameprofile.id).is_empty());
        assert_eq!(subject.player.config.load().skin_parts, 0x7e);

        let id = subject.player.gameprofile.id;
        scenario.leave(&subject).await;
        observer.packets();
        subject.send(&scenario.server, &settings(0x3f));
        assert!(hats(&observer.packets(), id).is_empty());
        let replacement = scenario.join(id, "HatControl").await;
        observer.packets();
        replacement
            .client()
            .handle_play_packet(
                &subject.player,
                &scenario.server,
                &pumpkin_protocol::RawPacket {
                    id: SClientInformationPlay::to_id(CURRENT_MC_VERSION),
                    payload: payload.into(),
                },
            )
            .unwrap();
        assert!(hats(&observer.packets(), id).is_empty());
        assert_eq!(replacement.player.config.load().skin_parts, 0x7f);
        scenario.stop().await;
    })
    .await;
}

struct RestoreHat {
    fired: AtomicBool,
}

impl EventHandler<PlayerLocaleChangeEvent> for RestoreHat {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PlayerLocaleChangeEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if self.fired.swap(true, Ordering::AcqRel) {
                return;
            }
            let mut packet = settings(0x7f);
            packet.locale = &event.new_locale;
            let mut payload = Vec::new();
            packet
                .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
                .unwrap();
            event
                .player
                .client
                .java()
                .unwrap()
                .handle_play_packet(
                    &event.player,
                    server,
                    &pumpkin_protocol::RawPacket {
                        id: SClientInformationPlay::to_id(CURRENT_MC_VERSION),
                        payload: payload.into(),
                    },
                )
                .unwrap();
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g07_player_list_hat_reentrant_settings_publish_live_value() {
    run(async {
        let scenario = Scenario::new().await;
        let subject = scenario.join(Uuid::new_v4(), "HatReenter").await;
        let observer = scenario.join(Uuid::new_v4(), "HatCallback").await;
        let hook = Arc::new(RestoreHat {
            fired: AtomicBool::new(false),
        });
        scenario
            .server
            .plugin_manager
            .register(hook.clone(), EventPriority::Normal, true);
        subject.packets();
        observer.packets();
        let mut packet = settings(0x3f);
        packet.locale = "fr_fr";
        subject.send(&scenario.server, &packet);
        assert!(hook.fired.load(Ordering::Acquire));
        assert_eq!(subject.player.config.load().skin_parts, 0x7f);
        for session in [&subject, &observer] {
            assert_hat_update(session, subject.player.gameprofile.id, true);
        }
        scenario.stop().await;
    })
    .await;
}
