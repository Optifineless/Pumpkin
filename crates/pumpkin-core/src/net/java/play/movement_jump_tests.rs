#![expect(
    clippy::expect_used,
    reason = "Movement regressions require a valid fixture and bounded setup/shutdown"
)]

use std::{
    num::NonZeroU8,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};

use pumpkin_data::Block;
use pumpkin_protocol::java::server::play::{
    FLAG_ON_GROUND, SPlayerPosition, SPlayerPositionRotation,
};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;

use crate::{
    entity::{
        EntityBase,
        death_test_world::DeathTestWorld,
        player::{Player, statistics::CustomStatistic},
    },
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, entity::entity_exhaustion::EntityExhaustionEvent,
    },
    server::Server,
    world::World,
};

async fn test_world() -> DeathTestWorld {
    tokio::time::timeout(Duration::from_secs(20), DeathTestWorld::new())
        .await
        .expect("movement fixture setup must finish")
}

async fn shutdown(fixture: &DeathTestWorld) {
    tokio::time::timeout(Duration::from_secs(20), fixture.server.shutdown())
        .await
        .expect("movement fixture shutdown must finish");
}

fn grounded_player(world: &Arc<World>, sprinting: bool) -> TestPlayer {
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(8, 63, 8, Block::STONE.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let fixture = TestPlayer::new(world);
    // Keep movement in the fixture's existing view, without starting chunk watch tasks.
    fixture.player.config.rcu(|config| {
        let mut config = (**config).clone();
        config.view_distance = NonZeroU8::new(2).expect("nonzero fixture view distance");
        config
    });
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    fixture.player.get_entity().on_ground.store(true, Relaxed);
    fixture.player.set_sprinting(sprinting);
    fixture
}

fn move_up(fixture: &TestPlayer, server: &Arc<Server>, y: f64, on_ground: bool, turning: bool) {
    send_up(&fixture.player, server, y, on_ground, turning);

    // Establish packet acceptance and matching geometry before testing accounting.
    let entity = fixture.player.get_entity();
    assert_eq!(entity.pos.load(), Vector3::new(8.5, y, 8.5));
    assert_eq!(entity.on_ground.load(Relaxed), on_ground);
    if turning {
        assert_eq!(entity.yaw.load(), 90.0);
        assert_eq!(entity.pitch.load(), 20.0);
    }
}

fn send_up(player: &Arc<Player>, server: &Arc<Server>, y: f64, on_ground: bool, turning: bool) {
    let client = player.client.java().expect("movement fixture uses Java");
    let position = Vector3::new(8.5, y, 8.5);
    let collision = if on_ground { FLAG_ON_GROUND } else { 0 };
    if turning {
        client.handle_position_rotation(
            player,
            server,
            &SPlayerPositionRotation {
                position,
                yaw: 90.0,
                pitch: 20.0,
                collision,
            },
        );
    } else {
        client.handle_position(
            player,
            server,
            &SPlayerPosition {
                position,
                collision,
            },
        );
    }
}

fn assert_accounting(fixture: &TestPlayer, jumps: i32, exhaustion: f32) {
    assert_eq!(fixture.player.get_custom_stat(CustomStatistic::Jump), jumps);
    assert!(
        (fixture.player.get_exhaustion() - exhaustion).abs() < 1.0E-6,
        "expected exhaustion {exhaustion}, got {}",
        fixture.player.get_exhaustion()
    );
}

async fn jump_accounts_once(turning: bool) {
    let fixture = test_world().await;
    for sprinting in [false, true] {
        let player = grounded_player(&fixture.world(), sprinting);
        let exhaustion = if sprinting { 0.2 } else { 0.05 };
        move_up(&player, &fixture.server, 64.42, false, turning);
        assert_accounting(&player, 1, exhaustion);
        move_up(&player, &fixture.server, 64.75, false, turning);
        assert_accounting(&player, 1, exhaustion);
    }
    shutdown(&fixture).await;
}

async fn upward_grounded_is_not_jump(turning: bool) {
    let fixture = test_world().await;
    for sprinting in [false, true] {
        let player = grounded_player(&fixture.world(), sprinting);
        move_up(&player, &fixture.server, 64.42, true, turning);
        assert_accounting(&player, 0, 0.0);
    }
    shutdown(&fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_position_jump_accounts_once_walking_and_sprinting() {
    jump_accounts_once(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_turning_jump_accounts_once_walking_and_sprinting() {
    jump_accounts_once(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_position_upward_grounded_movement_is_not_jump() {
    upward_grounded_is_not_jump(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_turning_upward_grounded_movement_is_not_jump() {
    upward_grounded_is_not_jump(true).await;
}

struct CancelExhaustion(Arc<AtomicUsize>);

impl EventHandler<EntityExhaustionEvent> for CancelExhaustion {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EntityExhaustionEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Relaxed);
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_jump_exhaustion_cancellation_keeps_jump_statistic() {
    let fixture = test_world().await;
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<EntityExhaustionEvent, _>(
            Arc::new(CancelExhaustion(calls.clone())),
            EventPriority::Normal,
            true,
        );
    for turning in [false, true] {
        for sprinting in [false, true] {
            let player = grounded_player(&fixture.world(), sprinting);
            let before = calls.load(Relaxed);
            move_up(&player, &fixture.server, 64.42, false, turning);
            assert_accounting(&player, 1, 0.0);
            assert_eq!(calls.load(Relaxed), before + 1);
            move_up(&player, &fixture.server, 64.75, false, turning);
            assert_accounting(&player, 1, 0.0);
            assert_eq!(calls.load(Relaxed), before + 1);
        }
    }
    shutdown(&fixture).await;
}

struct ReenterTakeoff {
    player: Arc<Player>,
    turning: bool,
    calls: AtomicUsize,
}

impl EventHandler<EntityExhaustionEvent> for ReenterTakeoff {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut EntityExhaustionEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_id == self.player.entity_id() && self.calls.fetch_add(1, Relaxed) == 0 {
            send_up(&self.player, server, 64.75, false, self.turning);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_r3_reentrant_exhaustion_counts_one_takeoff() {
    let fixture = test_world().await;
    for turning in [false, true] {
        for nested_turning in [false, true] {
            for sprinting in [false, true] {
                let player = grounded_player(&fixture.world(), sprinting);
                let callback = Arc::new(ReenterTakeoff {
                    player: player.player.clone(),
                    turning: nested_turning,
                    calls: AtomicUsize::new(0),
                });
                fixture
                    .server
                    .plugin_manager
                    .register::<EntityExhaustionEvent, _>(
                        callback.clone(),
                        EventPriority::Normal,
                        true,
                    );
                send_up(&player.player, &fixture.server, 64.42, false, turning);
                assert_accounting(&player, 1, if sprinting { 0.2 } else { 0.05 });
                assert_eq!(callback.calls.load(Relaxed), 1);
                assert_eq!(
                    player.player.get_entity().pos.load(),
                    Vector3::new(8.5, 64.75, 8.5)
                );
                assert!(!player.player.get_entity().on_ground.load(Relaxed));
                assert!(
                    player
                        .player
                        .awaiting_teleport
                        .lock()
                        .expect("teleport lock")
                        .is_none()
                );
                assert!(!player.player.client.closed());
            }
        }
    }
    shutdown(&fixture).await;
}
