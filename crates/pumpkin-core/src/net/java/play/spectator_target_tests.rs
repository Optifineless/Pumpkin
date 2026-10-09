use super::*;
use crate::{entity::death_test_world::DeathTestWorld, net::java::combat_test_support::TestPlayer};
use pumpkin_data::entity::EntityType;
use pumpkin_protocol::{VarInt, java::client::play::CSetCamera};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spectator_target_requires_border_range_and_pickability_and_sends_one_camera_packet() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mut viewer = TestPlayer::new(&world);
    viewer.player.set_gamemode(GameMode::Spectator);
    viewer
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 0.0));
    viewer.take_packets();
    let target = fixture.mob(&EntityType::COW);
    let action = SSpectatorAction {
        target: VarInt(target.get_entity().entity_id + 1),
    };
    target.get_entity().set_pos(Vector3::new(100.0, 64.0, 0.0));
    viewer
        .client()
        .handle_spectate_entity(&viewer.player, &action);
    assert!(viewer.player.camera_target_id.load().is_none());
    target.get_entity().set_pos(Vector3::new(2.0, 64.0, 0.0));
    world.worldborder.lock().unwrap().new_diameter = 2.0;
    viewer
        .client()
        .handle_spectate_entity(&viewer.player, &action);
    assert!(viewer.player.camera_target_id.load().is_none());
    world.worldborder.lock().unwrap().new_diameter = 1000.0;
    viewer
        .client()
        .handle_spectate_entity(&viewer.player, &action);
    let camera = viewer
        .client()
        .serialize_packet(&CSetCamera::new(target.get_entity().entity_id.into()))
        .unwrap();
    assert_eq!(
        viewer
            .take_packets()
            .iter()
            .filter(|packet| **packet == camera)
            .count(),
        1
    );
    assert_eq!(
        viewer.player.camera_target_id.load(),
        Some(target.get_entity().entity_id)
    );
    viewer.player.set_client_loaded(true);
    viewer
        .client()
        .handle_spectate_entity(&viewer.player, &action);
    assert_eq!(
        viewer
            .take_packets()
            .iter()
            .filter(|packet| **packet == camera)
            .count(),
        0
    );
    viewer.player.reset_camera();
    viewer.player.set_client_loaded(true);
    // An unpickable small fireball must not become a camera target.
    let fireball = fixture.mob(&EntityType::SMALL_FIREBALL);
    fireball.get_entity().set_pos(Vector3::new(2.0, 64.0, 0.0));
    viewer.client().handle_spectate_entity(
        &viewer.player,
        &SSpectatorAction {
            target: VarInt(fireball.get_entity().entity_id + 1),
        },
    );
    assert!(viewer.player.camera_target_id.load().is_none());
    viewer
        .client()
        .handle_spectate_entity(&viewer.player, &SSpectatorAction { target: VarInt(0) });
    assert!(viewer.player.camera_target_id.load().is_none());
    let dragon = fixture.mob(&EntityType::ENDER_DRAGON);
    let dragon = dragon
        .cast_any()
        .downcast_ref::<crate::entity::boss::ender_dragon::EnderDragonEntity>()
        .unwrap();
    let part = &dragon.parts[0];
    part.get_entity().set_pos(Vector3::new(2.0, 64.0, 0.0));
    viewer.player.set_client_loaded(true);
    viewer.client().handle_spectate_entity(
        &viewer.player,
        &SSpectatorAction {
            target: VarInt(part.get_entity().entity_id + 1),
        },
    );
    assert_eq!(
        viewer.player.camera_target_id.load(),
        Some(part.get_entity().entity_id)
    );
    fixture.server.shutdown().await;
}
