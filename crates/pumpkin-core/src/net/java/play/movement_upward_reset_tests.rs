#![expect(
    clippy::unwrap_used,
    reason = "Upward movement regression fixtures must be valid"
)]

use super::movement_landing_tests::{MovementTest, on_damage};
use crate::{
    entity::{EntityBase, player::Player},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        entity::entity_exhaustion::EntityExhaustionEvent,
        player::{
            player_move::PlayerMoveEvent, player_statistic_increment::PlayerStatisticIncrementEvent,
        },
    },
    server::Server,
};
use pumpkin_data::{Block, statistic::CustomStatistic};
use pumpkin_protocol::java::server::play::{
    FLAG_ON_GROUND, SPlayerPosition, SPlayerPositionRotation,
};
use pumpkin_util::math::vector3::Vector3;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
};

fn send_position(fixture: &MovementTest, y: f64, ground: bool, turning: bool) {
    let position = Vector3::new(4.5, y, 4.5);
    let collision = if ground { FLAG_ON_GROUND } else { 0 };
    if turning {
        fixture.client.client().handle_position_rotation(
            fixture.player(),
            &fixture.server,
            &SPlayerPositionRotation {
                position,
                yaw: 25.0,
                pitch: 10.0,
                collision,
            },
        );
    } else {
        fixture.client.client().handle_position(
            fixture.player(),
            &fixture.server,
            &SPlayerPosition {
                position,
                collision,
            },
        );
    }
}

fn accepted_upward_motion_resets_old_fall(turning: bool) {
    let fixture = MovementTest::new(&Block::STONE, false);
    fixture.position(1.0, false, turning);
    assert_eq!(fixture.player().living_entity.fall_distance.load(), 4.0);
    fixture.position(2.0, false, turning);
    assert_eq!(
        fixture.player().living_entity.fall_distance.load(),
        0.0,
        "accepted upward movement must discard the earlier four-block descent"
    );
    fixture.position(1.0, true, turning);
    fixture.assert_landed(20.0, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_accepted_upward_motion_resets_old_fall_position() {
    accepted_upward_motion_resets_old_fall(false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_accepted_upward_motion_resets_old_fall_position_rotation() {
    accepted_upward_motion_resets_old_fall(true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_continuous_five_block_descent_counts_final_packet() {
    for turning in [false, true] {
        let fixture = MovementTest::new(&Block::STONE, false);
        fixture
            .player()
            .get_entity()
            .set_pos(Vector3::new(4.5, 6.0, 4.5));
        fixture.position(1.6, false, turning);
        assert!((fixture.player().living_entity.fall_distance.load() - 4.4).abs() < 1.0E-6);
        fixture.position(1.0, true, turning);
        fixture.assert_landed(18.0, 500);
        fixture.position(1.0, true, turning);
        fixture.assert_landed(18.0, 500);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_unsupported_zero_movement_preserves_fall() {
    for turning in [false, true] {
        let fixture = MovementTest::new(&Block::STONE, false);
        fixture
            .player()
            .get_entity()
            .set_pos(Vector3::new(4.5, 6.0, 4.5));
        fixture.position(2.0, false, turning);
        fixture.position(2.0, true, turning);
        assert!(
            fixture
                .player()
                .get_entity()
                .supporting_block_pos
                .load()
                .is_none()
        );
        assert_eq!(fixture.player().living_entity.fall_distance.load(), 4.0);
        assert_eq!(fixture.player().living_entity.health.load(), 20.0);
        assert_eq!(
            fixture.player().get_custom_stat(CustomStatistic::FallOneCm),
            0
        );
        fixture.position(1.0, true, turning);
        fixture.assert_landed(18.0, 500);
    }
}

struct CancelMove {
    active: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}

impl EventHandler<PlayerMoveEvent> for CancelMove {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerMoveEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Relaxed);
        if self.active.load(Relaxed) {
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

fn unadmitted_upward_movement(turning: bool, gate: usize) {
    let fixture = MovementTest::new(&Block::STONE, false);
    fixture.position(1.0, false, turning);
    let cancel = Arc::new(AtomicBool::new(gate == 3));
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PlayerMoveEvent, _>(
            Arc::new(CancelMove {
                active: cancel.clone(),
                calls: calls.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    let player = fixture.player();
    match gate {
        0 => player.is_movement_locked.store(true, Relaxed),
        1 => {
            *player.awaiting_teleport.lock().unwrap() =
                Some((1.into(), Vector3::new(4.5, 1.0, 4.5)));
        }
        2 => player.set_client_loaded(false),
        _ => {}
    }
    send_position(&fixture, 2.0, false, turning);
    assert_eq!(player.get_entity().pos.load(), Vector3::new(4.5, 1.0, 4.5));
    assert_eq!(calls.load(Relaxed), usize::from(gate == 3));
    assert_eq!(player.living_entity.fall_distance.load(), 4.0);
    assert_eq!(player.living_entity.health.load(), 20.0);
    cancel.store(false, Relaxed);
    player.is_movement_locked.store(false, Relaxed);
    *player.awaiting_teleport.lock().unwrap() = None;
    player.set_client_loaded(true);
    fixture.position(1.0, true, turning);
    fixture.assert_landed(19.0, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_unadmitted_upward_motion_preserves_old_fall() {
    for turning in [false, true] {
        for gate in 0..4 {
            unadmitted_upward_movement(turning, gate);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_grounded_upward_motion_keeps_landing_order() {
    for turning in [false, true] {
        let fixture = MovementTest::new(&Block::STONE, false);
        fixture.position(1.0, false, turning);
        fixture.position(2.0, true, turning);
        fixture.assert_landed(19.0, 400);
    }
}

struct ResetOnExhaustion {
    player: Arc<Player>,
    calls: Arc<AtomicUsize>,
}

struct ResetOnMovementStatistic(Arc<AtomicUsize>);

impl EventHandler<PlayerStatisticIncrementEvent> for ResetOnMovementStatistic {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(self.0.fetch_add(1, Relaxed), 0);
        assert_eq!(event.amount, 100);
        event.player.living_entity.reset_state();
        event.player.set_health(17.0);
        event.player.living_entity.fall_distance.store(7.0);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_movement_statistic_reset_keeps_replacement_fall_state() {
    for turning in [false, true] {
        let fixture = MovementTest::new(&Block::STONE, false);
        fixture.position(1.0, false, turning);
        let calls = Arc::new(AtomicUsize::new(0));
        fixture
            .server
            .plugin_manager
            .register::<PlayerStatisticIncrementEvent, _>(
                Arc::new(ResetOnMovementStatistic(calls.clone())),
                EventPriority::Normal,
                true,
            );
        fixture.position(2.0, false, turning);
        assert_eq!(calls.load(Relaxed), 1);
        assert_eq!(fixture.player().living_entity.health.load(), 17.0);
        assert_eq!(
            fixture.player().living_entity.fall_distance.load(),
            7.0,
            "the movement statistic callback replaced the admitted life before fall accounting"
        );
    }
}

impl EventHandler<EntityExhaustionEvent> for ResetOnExhaustion {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityExhaustionEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(self.calls.fetch_add(1, Relaxed), 0);
        let player = &self.player;
        assert_eq!(event.entity_id, player.entity_id());
        player.living_entity.reset_state();
        player.set_health(17.0);
        player.living_entity.fall_distance.store(7.0);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_upward_cleanup_cannot_reset_replacement_life() {
    for turning in [false, true] {
        let fixture = MovementTest::new(&Block::STONE, false);
        fixture.position(1.0, false, turning);
        let calls = Arc::new(AtomicUsize::new(0));
        fixture
            .server
            .plugin_manager
            .register::<EntityExhaustionEvent, _>(
                Arc::new(ResetOnExhaustion {
                    player: fixture.player().clone(),
                    calls: calls.clone(),
                }),
                EventPriority::Normal,
                true,
            );
        // Swimming accounts vertical movement too; water contact alone counts horizontal motion.
        fixture.player().get_entity().set_swimming(true);
        fixture.position(2.0, false, turning);
        assert_eq!(calls.load(Relaxed), 1);
        assert_eq!(fixture.player().living_entity.health.load(), 17.0);
        assert_eq!(fixture.player().living_entity.fall_distance.load(), 7.0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_upward_landing_cleanup_preserves_reentrant_descent() {
    let fixture = MovementTest::new(&Block::STONE, false);
    let player = fixture.player().clone();
    let server = fixture.server.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    on_damage(&fixture.server, move |_| {
        assert_eq!(seen.fetch_add(1, Relaxed), 0);
        let client = player.client.java().unwrap();
        for y in [6.0, 2.0] {
            let position = Vector3::new(4.5, y, 4.5);
            client.handle_position(
                &player,
                &server,
                &SPlayerPosition {
                    position,
                    collision: 0,
                },
            );
            assert_eq!(player.get_entity().pos.load(), position);
        }
    });
    fixture.position(1.0, false, false);
    send_position(&fixture, 2.0, true, false);
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(
        fixture.player().get_entity().pos.load(),
        Vector3::new(4.5, 2.0, 4.5)
    );
    assert!(!fixture.player().get_entity().on_ground.load(Relaxed));
    assert_eq!(fixture.player().living_entity.health.load(), 19.0);
    assert_eq!(fixture.player().living_entity.fall_distance.load(), 4.0);
}

enum FallReplacement {
    Movement,
    PluginWrites,
}

struct ReplaceDescentOnStatistic {
    turning: bool,
    replacement: FallReplacement,
    injected: Arc<AtomicBool>,
}

impl EventHandler<PlayerStatisticIncrementEvent> for ReplaceDescentOnStatistic {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        if !self.injected.swap(true, Relaxed) {
            assert_eq!(event.amount, 100);
            let player = &event.player;
            if matches!(self.replacement, FallReplacement::PluginWrites) {
                player.living_entity.fall_distance.store(0.0);
                player.living_entity.fall_distance.store(4.0);
                return Box::pin(async {});
            }
            let client = player.client.java().unwrap();
            for (y, distance) in [(6.0, 0.0), (2.0, 4.0)] {
                let position = Vector3::new(4.5, y, 4.5);
                if self.turning {
                    client.handle_position_rotation(
                        player,
                        server,
                        &SPlayerPositionRotation {
                            position,
                            yaw: 25.0,
                            pitch: 10.0,
                            collision: 0,
                        },
                    );
                } else {
                    client.handle_position(
                        player,
                        server,
                        &SPlayerPosition {
                            position,
                            collision: 0,
                        },
                    );
                }
                assert_eq!(player.get_entity().pos.load(), position);
                assert_eq!(player.living_entity.fall_distance.load(), distance);
            }
        }
        Box::pin(async {})
    }
}

fn upward_cleanup_preserves_equal_reentrant_descent(turning: bool, replacement: FallReplacement) {
    let fixture = MovementTest::new(&Block::STONE, false);
    fixture.position(1.0, false, turning);
    assert_eq!(fixture.player().living_entity.fall_distance.load(), 4.0);
    let injected = Arc::new(AtomicBool::new(false));
    fixture
        .server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(ReplaceDescentOnStatistic {
                turning,
                replacement,
                injected: injected.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    let damage_calls = Arc::new(AtomicUsize::new(0));
    let seen = damage_calls.clone();
    on_damage(&fixture.server, move |event| {
        assert_eq!(event.damage_type, pumpkin_data::damage::DamageType::FALL);
        assert_eq!(event.damage, 2.0);
        seen.fetch_add(1, Relaxed);
    });
    fixture.position(2.0, false, turning);
    assert!(injected.load(Relaxed));
    assert_eq!(fixture.player().living_entity.fall_distance.load(), 4.0);
    assert_eq!(fixture.player().living_entity.health.load(), 20.0);
    assert_eq!(damage_calls.load(Relaxed), 0);
    fixture.position(1.0, true, turning);
    fixture.assert_landed(18.0, 500);
    assert_eq!(damage_calls.load(Relaxed), 1);
    fixture.position(1.0, true, turning);
    fixture.assert_landed(18.0, 500);
    assert_eq!(damage_calls.load(Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_upward_cleanup_preserves_equal_reentrant_descent_position() {
    upward_cleanup_preserves_equal_reentrant_descent(false, FallReplacement::Movement);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_upward_cleanup_preserves_equal_reentrant_descent_position_rotation() {
    upward_cleanup_preserves_equal_reentrant_descent(true, FallReplacement::Movement);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_upward_cleanup_preserves_equal_reentrant_descent_plugin_writes() {
    for turning in [false, true] {
        upward_cleanup_preserves_equal_reentrant_descent(turning, FallReplacement::PluginWrites);
    }
}
