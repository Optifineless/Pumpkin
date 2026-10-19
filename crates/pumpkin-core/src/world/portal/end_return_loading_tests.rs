#![expect(clippy::unwrap_used, reason = "Bounded real End lookup regressions")]

use std::sync::{Arc, Mutex, atomic::Ordering};

use pumpkin_data::dimension::Dimension;
use pumpkin_protocol::bedrock::client::{CChangeDimension, CShowCredits};
use pumpkin_util::math::vector3::Vector3;
use tokio::sync::oneshot;

use super::{
    end_return_test_support::{Arrived, DEADLINE, Fixture, PORTAL, contact, load_personal},
    end_return_tests::{assert_anchor, drain, install_anchor},
};
use crate::{
    entity::{EntityBase, player::respawn_position_test_gate},
    net::bedrock::combat_test_support::TestBedrockPlayer,
    plugin::{EventPriority, api::events::player::player_changed_world::PlayerChangedWorldEvent},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_saved_spawn_change_during_chunk_lookup_cannot_commit() {
    let fixture = Fixture::new().await;
    install_anchor(&fixture.nether);
    let mut client = fixture.returning_player();
    let player = client.player.clone();
    let initial = player.position();
    load_personal(&player, &Dimension::THE_NETHER, false);
    let (suspended, resume) = respawn_position_test_gate::install(player.gameprofile.id);
    contact(&fixture.server, player.as_ref());
    tokio::time::timeout(DEADLINE, suspended)
        .await
        .unwrap()
        .unwrap();
    load_personal(&player, &Dimension::OVERWORLD, false);
    let changed = player.respawn_point.lock().unwrap().clone();
    resume.send(()).unwrap();
    drain(&fixture, &mut client).await;
    assert!(Arc::ptr_eq(&player.world(), &fixture.end));
    assert_eq!(player.position(), initial);
    assert_eq!(*player.respawn_point.lock().unwrap(), changed);
    assert!(
        fixture
            .end
            .get_player_by_uuid(player.gameprofile.id)
            .is_some_and(|current| Arc::ptr_eq(&current, &player))
    );
    assert!(
        fixture
            .nether
            .get_player_by_uuid(player.gameprofile.id)
            .is_none()
    );
    assert_anchor(&fixture.nether);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_destination_unloaded_during_chunk_lookup_cannot_receive_player() {
    let fixture = Fixture::new().await;
    install_anchor(&fixture.nether);
    let mut client = fixture.returning_player();
    let player = client.player.clone();
    load_personal(&player, &Dimension::THE_NETHER, false);
    let before = player.respawn_point.lock().unwrap().clone();
    let (suspended, resume) = respawn_position_test_gate::install(player.gameprofile.id);
    contact(&fixture.server, player.as_ref());
    tokio::time::timeout(DEADLINE, suspended)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(
        DEADLINE,
        fixture.server.unload_world("minecraft:the_nether"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(fixture.nether.level.cancel_token.is_cancelled());
    resume.send(()).unwrap();
    drain(&fixture, &mut client).await;
    assert!(!Arc::ptr_eq(&player.world(), &fixture.nether));
    assert!(
        fixture
            .nether
            .get_player_by_uuid(player.gameprofile.id)
            .is_none()
    );
    assert!(!player.world().level.cancel_token.is_cancelled());
    assert_eq!(*player.respawn_point.lock().unwrap(), before);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_bedrock_end_return_transfers_to_personal_anchor() {
    let fixture = Fixture::new().await;
    install_anchor(&fixture.nether);
    let mut client = tokio::time::timeout(DEADLINE, TestBedrockPlayer::new(&fixture.end))
        .await
        .unwrap();
    let player = client.player.clone();
    player.get_entity().set_pos(PORTAL.to_centered_f64());
    player.seen_credits.store(true, Ordering::Relaxed);
    load_personal(&player, &Dimension::THE_NETHER, false);
    let (done, completed) = oneshot::channel();
    fixture
        .server
        .plugin_manager
        .register::<PlayerChangedWorldEvent, _>(
            Arc::new(Arrived(Mutex::new(Some(done)))),
            EventPriority::Lowest,
            true,
        );
    contact(&fixture.server, player.as_ref());
    tokio::time::timeout(DEADLINE, completed)
        .await
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&player.world(), &fixture.nether));
    assert_eq!(player.position(), Vector3::new(8.5, 64.0, 7.5));
    assert!(player.get_entity().yaw.load().abs() < 0.001);
    assert_anchor(&fixture.nether);
    let change_dimension = client
        .client()
        .serialize_packet(&CChangeDimension {
            dimension_id: 1.into(),
            position: Vector3::new(8.5, 64.0, 7.5),
            respawn: false,
            loading_screen_id: None,
        })
        .unwrap();
    let credits = client
        .client()
        .serialize_packet(&CShowCredits {
            player_runtime_id: (player.get_entity().entity_id as u64).into(),
            credits_state: 0.into(),
        })
        .unwrap();
    let packets = client.take_packets();
    assert!(packets.contains(&change_dimension));
    assert!(packets.contains(&credits));
    tokio::time::timeout(DEADLINE, client.close())
        .await
        .unwrap();
    fixture.finish().await;
}
