#![expect(
    clippy::unwrap_used,
    reason = "Movement regression fixtures must be valid"
)]

use crate::{
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, Payload, entity::entity_damage::EntityDamageEvent,
    },
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::{Block, damage::DamageType, statistic::CustomStatistic};
use pumpkin_protocol::java::server::play::{
    FLAG_ON_GROUND, SPlayerPosition, SPlayerPositionRotation, SPlayerRotation, SSetPlayerGround,
};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};
use pumpkin_world::chunk::ChunkData;
use std::{
    num::NonZeroU8,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
};

pub(super) struct MovementTest {
    pub(super) server: Arc<Server>,
    pub(super) world: Arc<World>,
    pub(super) client: TestPlayer,
    _directory: tempfile::TempDir,
}

impl MovementTest {
    pub(super) fn new(support: &'static Block, adjacent_slime: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        let chunk = ChunkData::empty_sync(0, 0);
        chunk.set_block_absolute_y(4, 0, 4, support.default_state.id);
        if adjacent_slime {
            chunk.set_block_absolute_y(5, 0, 4, Block::SLIME_BLOCK.default_state.id);
        }
        world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
        let client = TestPlayer::new(&world);
        client.player.config.rcu(|config| {
            let mut config = (**config).clone();
            config.view_distance = NonZeroU8::new(2).unwrap();
            config
        });
        client
            .player
            .get_entity()
            .set_pos(Vector3::new(4.5, 5.0, 4.5));
        client.player.get_entity().on_ground.store(false, Relaxed);
        Self {
            server,
            world,
            client,
            _directory: directory,
        }
    }

    pub(super) fn player(&self) -> &Arc<Player> {
        &self.client.player
    }

    pub(super) fn position(&self, y: f64, ground: bool, rotation: bool) {
        let position = Vector3::new(4.5, y, 4.5);
        let collision = if ground { FLAG_ON_GROUND } else { 0 };
        if rotation {
            self.client.client().handle_position_rotation(
                self.player(),
                &self.server,
                &SPlayerPositionRotation {
                    position,
                    yaw: 25.0,
                    pitch: 10.0,
                    collision,
                },
            );
        } else {
            self.client.client().handle_position(
                self.player(),
                &self.server,
                &SPlayerPosition {
                    position,
                    collision,
                },
            );
        }
        self.assert_accepted(y, ground);
    }

    fn assert_accepted(&self, y: f64, ground: bool) {
        let entity = self.player().get_entity();
        assert_eq!(entity.pos.load(), Vector3::new(4.5, y, 4.5));
        assert_eq!(entity.on_ground.load(Relaxed), ground);
        assert!(entity.bounding_box.load().max.x < 5.0);
        assert!(self.player().awaiting_teleport.lock().unwrap().is_none());
        assert!(!self.player().client.closed());
    }

    pub(super) fn ground(&self, rotation: bool) {
        if rotation {
            self.client.client().handle_rotation(
                self.player(),
                &SPlayerRotation {
                    yaw: 30.0,
                    pitch: 15.0,
                    ground: true,
                },
            );
        } else {
            self.client
                .client()
                .handle_player_ground(self.player(), &SSetPlayerGround { on_ground: true });
        }
        self.assert_accepted(1.0, true);
    }

    pub(super) fn assert_landed(&self, health: f32, centimeters: i32) {
        assert_eq!(self.player().living_entity.health.load(), health);
        assert_eq!(self.player().living_entity.fall_distance.load(), 0.0);
        assert_eq!(
            self.player().get_custom_stat(CustomStatistic::FallOneCm),
            centimeters
        );
    }
}

fn final_descent(rotation: bool) {
    let fixture = MovementTest::new(&Block::STONE, false);
    fixture.position(1.6, false, rotation);
    assert!((fixture.player().living_entity.fall_distance.load() - 3.4).abs() < 1.0E-6);
    fixture.position(1.0, true, rotation);
    fixture.assert_landed(19.0, 400);
    fixture.position(1.0, true, rotation);
    fixture.assert_landed(19.0, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_final_position_descent_is_counted() {
    final_descent(false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_final_turning_descent_is_counted() {
    final_descent(true);
}

fn zero_delta_landing(rotation: bool) {
    let fixture = MovementTest::new(&Block::STONE, false);
    fixture.position(1.0, false, false);
    assert_eq!(fixture.player().living_entity.fall_distance.load(), 4.0);
    fixture.ground(rotation);
    fixture.assert_landed(19.0, 400);
    fixture.ground(rotation);
    fixture.assert_landed(19.0, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_ground_only_lands_once() {
    zero_delta_landing(false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_rotation_only_lands_once() {
    zero_delta_landing(true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_three_blocks_do_not_damage() {
    let fixture = MovementTest::new(&Block::STONE, false);
    fixture
        .player()
        .get_entity()
        .set_pos(Vector3::new(4.5, 4.0, 4.5));
    fixture.position(1.6, false, false);
    fixture.position(1.0, true, false);
    fixture.assert_landed(20.0, 300);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_noncontact_slime_does_not_exempt_stone() {
    for adjacent in [false, true] {
        let fixture = MovementTest::new(&Block::STONE, adjacent);
        fixture.position(1.0, false, false);
        assert_eq!(
            fixture.world.get_block(&BlockPos::new(4, 0, 4)),
            &Block::STONE
        );
        if adjacent {
            assert_eq!(
                fixture.world.get_block(&BlockPos::new(5, 0, 4)),
                &Block::SLIME_BLOCK
            );
        }
        assert_eq!(fixture.player().living_entity.fall_distance.load(), 4.0);
        // A zero-descending position landing isolates the support exemption from #112.
        fixture.position(1.0, true, false);
        fixture.assert_landed(19.0, 400);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_true_slime_keeps_support_callback_without_damage() {
    let fixture = MovementTest::new(&Block::SLIME_BLOCK, false);
    fixture.position(1.0, false, false);
    fixture.position(1.0, true, false);
    fixture.assert_landed(20.0, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_water_climb_and_scaffolding_keep_exemptions() {
    for block in [&Block::WATER, &Block::LADDER, &Block::SCAFFOLDING] {
        let fixture = MovementTest::new(&Block::STONE, false);
        fixture
            .world
            .level
            .loaded_chunks
            .get(&Vector2::new(0, 0))
            .unwrap()
            .set_block_absolute_y(4, 1, 4, block.default_state.id);
        if block == &Block::SCAFFOLDING {
            fixture.player().get_entity().set_sneaking(true);
        }
        fixture.position(1.0, false, false);
        fixture.position(1.0, true, false);
        fixture.assert_landed(20.0, 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_honey_support_keeps_reduced_damage_callback() {
    let fixture = MovementTest::new(&Block::HONEY_BLOCK, false);
    fixture
        .player()
        .get_entity()
        .set_pos(Vector3::new(4.5, 4.9375, 4.5));
    fixture.position(0.9375, false, false);
    fixture.position(0.9375, true, false);
    fixture.assert_landed(20.0, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_flight_creative_and_disabled_gamerule_keep_damage_gates() {
    for mode in 0..3 {
        let fixture = MovementTest::new(&Block::STONE, false);
        if mode == 0 {
            fixture.player().abilities.lock().unwrap().flying = true;
        } else if mode == 1 {
            fixture.player().gamemode.store(GameMode::Creative);
            fixture.player().abilities.lock().unwrap().allow_flying = true;
        } else {
            fixture.server.level_info.rcu(|info| {
                let mut info = (**info).clone();
                info.game_rules.fall_damage = false;
                info
            });
        }
        fixture.position(1.6, false, false);
        fixture.position(1.0, true, false);
        fixture.assert_landed(20.0, if mode == 2 { 400 } else { 0 });
    }
}

struct Callback<F>(F);
impl<E: Payload, F: Fn(&mut E) + Send + Sync> EventHandler<E> for Callback<F> {
    fn handle_blocking<'a>(&'a self, _: &'a Arc<Server>, event: &'a mut E) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

pub(super) fn on_damage(
    server: &Server,
    action: impl Fn(&mut EntityDamageEvent) + Send + Sync + 'static,
) {
    server.plugin_manager.register::<EntityDamageEvent, _>(
        Arc::new(Callback(action)),
        EventPriority::Normal,
        true,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_cancelled_damage_keeps_payload_and_resets_once() {
    let fixture = MovementTest::new(&Block::STONE, false);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let player = fixture.player().clone();
    on_damage(&fixture.server, move |event| {
        assert_eq!(event.entity_id, player.entity_id());
        assert_eq!(event.damage_type, DamageType::FALL);
        assert_eq!(event.damage, 1.0);
        assert_eq!(player.living_entity.fall_distance.load(), 4.0);
        assert!(player.get_entity().on_ground.load(Relaxed));
        seen.fetch_add(1, Relaxed);
        event.cancelled = true;
    });
    fixture.position(1.6, false, false);
    fixture.position(1.0, true, false);
    fixture.assert_landed(20.0, 400);
    fixture.ground(false);
    assert_eq!(calls.load(Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_reentrant_landing_does_not_consume_twice() {
    let fixture = MovementTest::new(&Block::STONE, false);
    let player = fixture.player().clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    on_damage(&fixture.server, move |_| {
        assert_eq!(seen.fetch_add(1, Relaxed), 0);
        if let crate::net::ClientPlatform::Java(client) = player.client.as_ref() {
            client.handle_player_ground(&player, &SSetPlayerGround { on_ground: true });
        }
    });
    fixture.position(1.6, false, false);
    fixture.position(1.0, true, false);
    fixture.assert_landed(19.0, 400);
    assert_eq!(calls.load(Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_landing_callback_cannot_reset_replacement_life() {
    let fixture = MovementTest::new(&Block::STONE, false);
    let player = fixture.player().clone();
    on_damage(&fixture.server, move |_| {
        player.living_entity.reset_state();
        player.living_entity.fall_distance.store(7.0);
    });
    fixture.position(1.6, false, false);
    fixture.position(1.0, true, false);
    assert_eq!(fixture.player().living_entity.health.load(), 20.0);
    assert_eq!(fixture.player().living_entity.fall_distance.load(), 7.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_callback_descent_does_not_recharge_consumed_distance() {
    let fixture = MovementTest::new(&Block::STONE, false);
    let player = fixture.player().clone();
    let server = fixture.server.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    on_damage(&fixture.server, move |_| {
        assert_eq!(seen.fetch_add(1, Relaxed), 0);
        if let crate::net::ClientPlatform::Java(client) = player.client.as_ref() {
            for y in [2.0, 1.5] {
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
                assert!(!player.get_entity().on_ground.load(Relaxed));
            }
        }
    });
    fixture.position(1.6, false, false);
    fixture.client.client().handle_position(
        fixture.player(),
        &fixture.server,
        &SPlayerPosition {
            position: Vector3::new(4.5, 1.0, 4.5),
            collision: FLAG_ON_GROUND,
        },
    );
    fixture.assert_accepted(1.5, false);
    assert_eq!(fixture.player().living_entity.fall_distance.load(), 0.5);
    assert_eq!(fixture.player().living_entity.health.load(), 19.0);
    fixture.position(1.0, true, false);
    fixture.assert_landed(19.0, 400);
    assert_eq!(calls.load(Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_unadmitted_zero_delta_packets_do_not_land() {
    for rotation in [false, true] {
        for gate in 0..3 {
            let fixture = MovementTest::new(&Block::STONE, false);
            fixture.position(1.0, false, false);
            if gate == 0 {
                fixture.player().is_movement_locked.store(true, Relaxed);
            } else if gate == 1 {
                *fixture.player().awaiting_teleport.lock().unwrap() =
                    Some((1.into(), Vector3::new(4.5, 1.0, 4.5)));
            } else {
                fixture.player().set_client_loaded(false);
            }
            if rotation {
                fixture.client.client().handle_rotation(
                    fixture.player(),
                    &SPlayerRotation {
                        yaw: 30.0,
                        pitch: 15.0,
                        ground: true,
                    },
                );
            } else {
                fixture
                    .client
                    .client()
                    .handle_player_ground(fixture.player(), &SSetPlayerGround { on_ground: true });
            }
            assert_eq!(fixture.player().living_entity.health.load(), 20.0);
            assert_eq!(fixture.player().living_entity.fall_distance.load(), 4.0);
            assert_eq!(
                fixture.player().get_custom_stat(CustomStatistic::FallOneCm),
                0
            );
        }
    }
}
