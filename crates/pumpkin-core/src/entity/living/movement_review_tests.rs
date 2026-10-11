#![expect(clippy::unwrap_used, reason = "Movement race fixtures must be valid")]

use crate::{
    block::{BlockBehaviour, OnLandedUponArgs, registry::BlockRegistry},
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::Block;
use pumpkin_macros::pumpkin_block;
use pumpkin_protocol::java::server::play::{
    FLAG_ON_GROUND, SPlayerPosition, SPlayerPositionRotation,
};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::sync::Arc;

const RESTORED_FALL_DISTANCE: f32 = 13.0;

fn replace_life(player: &Arc<Player>) {
    let living = &player.living_entity;
    let source = living.begin_respawn().unwrap();
    living.reset_state();
    assert!(living.publish_respawn(
        player,
        &source,
        &source,
        Vector3::new(4.5, 64.0, 4.5),
        0.0,
        0.0,
    ));
    living.fall_distance.store(RESTORED_FALL_DISTANCE);
    living.entity.velocity.store(Vector3::new(0.0, 0.75, 0.0));
}

#[pumpkin_block("minecraft:stone")]
struct LandingProbe;

impl BlockBehaviour for LandingProbe {
    fn on_landed_upon(&self, args: OnLandedUponArgs<'_>) {
        let player = args
            .world
            .get_player_by_id(args.entity.get_entity().entity_id)
            .unwrap();
        assert!(
            !player
                .living_entity
                .damage_owner
                .is_owned_by_current_thread(),
            "landing callback retained combat ownership"
        );
        replace_life(&player);
        player
            .living_entity
            .handle_fall_damage(args.entity, args.fall_distance, 1.0);
    }
}

fn move_player(fixture: &TestPlayer, server: &Arc<crate::server::Server>, rotation: bool) {
    let position = Vector3::new(4.5, 64.0, 4.5);
    if rotation {
        fixture.client().handle_position_rotation(
            &fixture.player,
            server,
            &SPlayerPositionRotation {
                position,
                yaw: 0.0,
                pitch: 0.0,
                collision: FLAG_ON_GROUND,
            },
        );
    } else {
        fixture.client().handle_position(
            &fixture.player,
            server,
            &SPlayerPosition {
                position,
                collision: FLAG_ON_GROUND,
            },
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review7_movement_landing_releases_ownership_and_preserves_replaced_life() {
    for rotation in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut server = server(dir.path());
        let mut registry = BlockRegistry::default();
        registry.register(LandingProbe);
        Arc::get_mut(&mut server).unwrap().block_registry = Arc::new(registry);
        let world = world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = fixture.player.clone();
        let chunk = ChunkData::empty_sync(0, 0);
        chunk.set_block_absolute_y(4, 63, 4, Block::STONE.default_state.id);
        world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
        player.get_entity().set_pos(Vector3::new(4.5, 67.0, 4.5));
        player.living_entity.fall_distance.store(10.0);
        move_player(&fixture, &server, rotation);
        assert_eq!(
            player.living_entity.fall_distance.load(),
            RESTORED_FALL_DISTANCE
        );
        assert_eq!(player.living_entity.health.load(), 20.0);
        assert_eq!(
            player.get_entity().velocity.load(),
            Vector3::new(0.0, 0.75, 0.0)
        );
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review7_movement_tracker_releases_ownership_and_stops_replaced_life() {
    for rotation in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = fixture.player.clone();
        player.get_entity().set_pos(Vector3::new(4.5, 67.0, 4.5));
        let mut config = (**player.config.load()).clone();
        config.view_distance = std::num::NonZeroU8::new(2).unwrap();
        player.config.store(Arc::new(config));
        crate::world::chunker::tests::on_next_tracker_walk(player.entity_id(), {
            let player = player.clone();
            move || {
                assert!(
                    !player
                        .living_entity
                        .damage_owner
                        .is_owned_by_current_thread(),
                    "tracker walk retained combat ownership"
                );
                replace_life(&player);
            }
        });
        move_player(&fixture, &server, rotation);
        assert_eq!(
            player.get_entity().velocity.load(),
            Vector3::new(0.0, 0.75, 0.0)
        );
    }
    crate::server::fixture_lifecycle::finish().await;
}
