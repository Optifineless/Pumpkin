#![expect(
    clippy::unwrap_used,
    reason = "Landing support regression fixtures must be valid"
)]

use super::movement_landing_tests::MovementTest;
use crate::entity::EntityBase;
use pumpkin_data::Block;
use pumpkin_protocol::java::server::play::{
    FLAG_ON_GROUND, SPlayerPosition, SPlayerPositionRotation, SPlayerRotation, SSetPlayerGround,
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use std::sync::atomic::Ordering::Relaxed;

#[derive(Clone, Copy)]
enum LandingPacket {
    Position,
    PositionRotation,
    Ground,
    Rotation,
}

fn movement(fixture: &MovementTest, x: f64, y: f64, ground: bool, packet: LandingPacket) {
    let position = Vector3::new(x, y, 4.5);
    let collision = if ground { FLAG_ON_GROUND } else { 0 };
    let client = fixture.client.client();
    let player = fixture.player();
    match packet {
        LandingPacket::Position => client.handle_position(
            player,
            &fixture.server,
            &SPlayerPosition {
                position,
                collision,
            },
        ),
        LandingPacket::PositionRotation => client.handle_position_rotation(
            player,
            &fixture.server,
            &SPlayerPositionRotation {
                position,
                yaw: 25.0,
                pitch: 10.0,
                collision,
            },
        ),
        LandingPacket::Ground => {
            client.handle_player_ground(player, &SSetPlayerGround { on_ground: ground });
        }
        LandingPacket::Rotation => client.handle_rotation(
            player,
            &SPlayerRotation {
                yaw: 30.0,
                pitch: 15.0,
                ground,
            },
        ),
    }
    assert_eq!(player.get_entity().pos.load(), position);
    assert_eq!(player.get_entity().on_ground.load(Relaxed), ground);
    assert!(player.awaiting_teleport.lock().unwrap().is_none());
    assert!(!player.client.closed());
}

fn edge_slime(health: f32) -> MovementTest {
    let fixture = MovementTest::new(&Block::SLIME_BLOCK, false);
    let player = fixture.player();
    player.get_entity().set_pos(Vector3::new(5.2, 5.0, 4.5));
    player.set_health(health);
    let bounds = player.get_entity().bounding_box.load();
    assert!(bounds.min.x < 5.0 && bounds.max.x > 5.0);
    assert_eq!(
        fixture.world.get_block(&BlockPos::new(5, 0, 4)),
        &Block::AIR
    );
    fixture
}

fn assert_safe_landing(fixture: &MovementTest, health: f32) {
    assert!(!fixture.player().living_entity.dead.load(Relaxed));
    assert!(!fixture.player().get_entity().is_removed());
    fixture.assert_landed(health, 400);
}

fn edge_descent(packet: LandingPacket, health: f32) {
    let fixture = edge_slime(health);
    movement(&fixture, 5.2, 1.6, false, packet);
    assert!((fixture.player().living_entity.fall_distance.load() - 3.4).abs() < 1.0E-6);
    movement(&fixture, 5.2, 1.0, true, packet);
    assert_safe_landing(&fixture, health);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r2_position_edge_slime_keeps_full_health() {
    edge_descent(LandingPacket::Position, 20.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r2_turning_edge_slime_keeps_full_health() {
    edge_descent(LandingPacket::PositionRotation, 20.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r2_position_edge_slime_does_not_kill() {
    edge_descent(LandingPacket::Position, 1.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r2_turning_edge_slime_does_not_kill() {
    edge_descent(LandingPacket::PositionRotation, 1.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r2_zero_delta_edge_slime_refreshes_support() {
    for packet in [
        LandingPacket::Position,
        LandingPacket::PositionRotation,
        LandingPacket::Ground,
        LandingPacket::Rotation,
    ] {
        for health in [20.0, 1.0] {
            let fixture = edge_slime(health);
            movement(&fixture, 5.2, 1.0, false, LandingPacket::Position);
            assert_eq!(fixture.player().living_entity.fall_distance.load(), 4.0);
            movement(&fixture, 5.2, 1.0, true, packet);
            assert_safe_landing(&fixture, health);
            movement(&fixture, 5.2, 1.0, true, packet);
            assert_safe_landing(&fixture, health);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r2_landing_replaces_stale_support() {
    for packet in [LandingPacket::Position, LandingPacket::PositionRotation] {
        let fixture = edge_slime(1.0);
        movement(&fixture, 5.2, 1.6, false, packet);
        fixture
            .world
            .level
            .loaded_chunks
            .get(&Vector2::new(0, 0))
            .unwrap()
            .set_block_absolute_y(6, 0, 4, Block::STONE.default_state.id);
        fixture
            .player()
            .get_entity()
            .supporting_block_pos
            .store(Some(BlockPos::new(6, 0, 4)));
        movement(&fixture, 5.2, 1.0, true, packet);
        assert_safe_landing(&fixture, 1.0);

        let fixture = MovementTest::new(&Block::STONE, true);
        fixture.position(
            1.6,
            false,
            matches!(packet, LandingPacket::PositionRotation),
        );
        fixture
            .player()
            .get_entity()
            .supporting_block_pos
            .store(Some(BlockPos::new(5, 0, 4)));
        fixture.position(1.0, true, matches!(packet, LandingPacket::PositionRotation));
        fixture.assert_landed(19.0, 400);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r2_nearer_noncolliding_slab_does_not_mask_slime() {
    assert!(
        Block::STONE_SLAB
            .default_state
            .get_block_collision_shapes()
            .all(|shape| shape.max.y <= 0.5)
    );
    for packet in [LandingPacket::Position, LandingPacket::PositionRotation] {
        let fixture = edge_slime(1.0);
        fixture
            .world
            .level
            .loaded_chunks
            .get(&Vector2::new(0, 0))
            .unwrap()
            .set_block_absolute_y(5, 0, 4, Block::STONE_SLAB.default_state.id);
        movement(&fixture, 5.2, 1.6, false, packet);
        movement(&fixture, 5.2, 1.0, true, packet);
        assert_safe_landing(&fixture, 1.0);
    }
}

fn moving_off_slime(packet: LandingPacket, health: f32) {
    let fixture = edge_slime(health);
    fixture
        .player()
        .get_entity()
        .set_pos(Vector3::new(5.11, 5.0, 4.5));
    movement(&fixture, 5.11, 1.6, false, packet);
    assert!((fixture.player().living_entity.fall_distance.load() - 3.4).abs() < 1.0E-6);
    movement(&fixture, 5.31, 1.0, true, packet);
    assert!(fixture.player().get_entity().bounding_box.load().min.x > 5.0);
    assert_safe_landing(&fixture, health);
    assert_eq!(
        fixture.player().get_entity().supporting_block_pos.load(),
        Some(BlockPos::new(4, 0, 4))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r3_position_moving_off_slime_keeps_full_health() {
    moving_off_slime(LandingPacket::Position, 20.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r3_turning_moving_off_slime_keeps_full_health() {
    moving_off_slime(LandingPacket::PositionRotation, 20.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r3_position_moving_off_slime_does_not_kill() {
    moving_off_slime(LandingPacket::Position, 1.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r3_turning_moving_off_slime_does_not_kill() {
    moving_off_slime(LandingPacket::PositionRotation, 1.0);
}

fn unsupported_ground_preserves_fall(packet: LandingPacket) {
    let fixture = MovementTest::new(&Block::STONE, false);
    fixture
        .player()
        .get_entity()
        .set_pos(Vector3::new(4.5, 9.0, 4.5));
    for (y, distance) in [(7.0, 2.0), (5.0, 4.0), (3.0, 6.0)] {
        movement(&fixture, 4.5, y, false, LandingPacket::Position);
        movement(&fixture, 4.5, y, true, packet);
        assert_eq!(
            fixture.player().get_entity().supporting_block_pos.load(),
            None
        );
        assert_eq!(
            fixture.player().living_entity.fall_distance.load(),
            distance
        );
        assert_eq!(fixture.player().living_entity.health.load(), 20.0);
        assert_eq!(
            fixture
                .player()
                .get_custom_stat(pumpkin_data::statistic::CustomStatistic::FallOneCm),
            0
        );
    }
    movement(&fixture, 4.5, 1.0, true, LandingPacket::Position);
    fixture.assert_landed(15.0, 800);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r3_flags_only_unsupported_ground_preserves_fall() {
    unsupported_ground_preserves_fall(LandingPacket::Ground);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r3_rotation_only_unsupported_ground_preserves_fall() {
    unsupported_ground_preserves_fall(LandingPacket::Rotation);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r3_same_position_unsupported_ground_preserves_fall() {
    unsupported_ground_preserves_fall(LandingPacket::Position);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r3_same_turning_position_unsupported_ground_preserves_fall() {
    unsupported_ground_preserves_fall(LandingPacket::PositionRotation);
}
