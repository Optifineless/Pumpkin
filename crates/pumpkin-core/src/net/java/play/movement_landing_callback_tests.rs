#![expect(
    clippy::unwrap_used,
    reason = "Movement callback regression fixtures must be valid"
)]

use super::movement_landing_tests::{MovementTest, on_damage};
use crate::{
    entity::{EntityBase, player::Player},
    net::{
        ClientPlatform, PlayerConfig,
        java::combat_test_support::{OutgoingPacket, captured_client},
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        player::player_statistic_increment::PlayerStatisticIncrementEvent,
    },
    server::Server,
};
use pumpkin_data::{Block, packet::clientbound::play::SOUND, statistic::CustomStatistic};
use pumpkin_protocol::{
    java::server::play::{FLAG_ON_GROUND, SPlayerPosition},
    ser::NetworkReadExt,
};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

struct StatisticCallback<F>(F);

impl<F: Fn(&mut PlayerStatisticIncrementEvent) + Send + Sync>
    EventHandler<PlayerStatisticIncrementEvent> for StatisticCallback<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        if event.statistic_id == format!("Custom:{}", CustomStatistic::FallOneCm as i32) {
            (self.0)(event);
        }
        Box::pin(async {})
    }
}

fn on_fall_statistic(
    server: &Server,
    action: impl Fn(&mut PlayerStatisticIncrementEvent) + Send + Sync + 'static,
) {
    server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(StatisticCallback(action)),
            EventPriority::Normal,
            true,
        );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_fall_statistic_callback_cannot_damage_replacement_life() {
    let fixture = MovementTest::new(&Block::STONE, false);
    let callbacks = Arc::new(AtomicUsize::new(0));
    let seen = callbacks.clone();
    on_fall_statistic(&fixture.server, move |event| {
        assert_eq!(event.amount, 400);
        assert_eq!(event.player.living_entity.fall_distance.load(), 4.0);
        assert_eq!(seen.fetch_add(1, Relaxed), 0);
        event.player.living_entity.reset_state();
        event.player.living_entity.fall_distance.store(7.0);
    });
    let damage_events = Arc::new(AtomicUsize::new(0));
    let seen = damage_events.clone();
    on_damage(&fixture.server, move |_| {
        seen.fetch_add(1, Relaxed);
    });

    fixture.position(1.6, false, false);
    fixture.position(1.0, true, false);
    assert_eq!(callbacks.load(Relaxed), 1);
    assert_eq!(damage_events.load(Relaxed), 0);
    assert_eq!(fixture.player().living_entity.health.load(), 20.0);
    assert_eq!(fixture.player().living_entity.fall_distance.load(), 7.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_fall_statistic_callback_cannot_damage_retired_session() {
    let fixture = MovementTest::new(&Block::STONE, false);
    let profile = fixture.player().gameprofile.clone();
    let (client, mut packets) = captured_client(profile.clone());
    let replacement = Arc::new(Player::new(
        Arc::new(ClientPlatform::Java(client)),
        profile,
        PlayerConfig::default(),
        &fixture.world,
        GameMode::Survival,
    ));
    replacement.set_client_loaded(true);
    replacement
        .get_entity()
        .set_pos(Vector3::new(4.5, 1.0, 4.5));
    replacement.living_entity.fall_distance.store(7.0);
    let current = replacement.clone();
    let world = fixture.world.clone();
    let callbacks = Arc::new(AtomicUsize::new(0));
    let seen = callbacks.clone();
    on_fall_statistic(&fixture.server, move |event| {
        assert_eq!(event.amount, 400);
        assert_eq!(event.player.living_entity.fall_distance.load(), 4.0);
        assert_eq!(seen.fetch_add(1, Relaxed), 0);
        world.players.store(Arc::new(vec![current.clone()]));
    });
    let damage_events = Arc::new(AtomicUsize::new(0));
    let seen = damage_events.clone();
    on_damage(&fixture.server, move |_| {
        seen.fetch_add(1, Relaxed);
    });

    fixture.position(1.6, false, false);
    fixture.position(1.0, true, false);
    let published = fixture
        .world
        .get_player_by_uuid(replacement.gameprofile.id)
        .unwrap();
    assert!(Arc::ptr_eq(&published, &replacement));
    assert!(!Arc::ptr_eq(&published, fixture.player()));
    assert!(replacement.has_client_loaded());
    assert!(!replacement.client.closed());
    assert_eq!(callbacks.load(Relaxed), 1);
    assert_eq!(damage_events.load(Relaxed), 0);
    assert_eq!(fixture.player().living_entity.health.load(), 20.0);
    assert_eq!(replacement.living_entity.health.load(), 20.0);
    assert_eq!(replacement.living_entity.fall_distance.load(), 7.0);
    // A retired landing must stop before its fall sound, even if damage admission rejects it.
    while let Ok(packet) = packets.try_recv() {
        if let OutgoingPacket::Data { data, .. } = packet {
            let mut bytes = data.as_ref();
            assert_ne!(bytes.get_var_int().unwrap().0, SOUND.0);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_accepted_honey_side_slide_resets_before_landing() {
    let fixture = MovementTest::new(&Block::STONE, false);
    let chunk = fixture
        .world
        .level
        .loaded_chunks
        .get(&Vector2::new(0, 0))
        .unwrap();
    chunk.set_block_absolute_y(5, 2, 4, Block::HONEY_BLOCK.default_state.id);
    chunk.set_block_absolute_y(6, 0, 4, Block::STONE.default_state.id);
    drop(chunk);
    fixture
        .player()
        .get_entity()
        .set_pos(Vector3::new(6.25, 5.5, 4.5));

    for (y, ground) in [(1.5, false), (1.0, true)] {
        let position = Vector3::new(6.25, y, 4.5);
        fixture.client.client().handle_position(
            fixture.player(),
            &fixture.server,
            &SPlayerPosition {
                position,
                collision: if ground { FLAG_ON_GROUND } else { 0 },
            },
        );
        let entity = fixture.player().get_entity();
        assert_eq!(entity.pos.load(), position);
        assert_eq!(entity.on_ground.load(Relaxed), ground);
        let bounds = entity.bounding_box.load();
        // Contact with honey's full inside volume, outside its inset physical collider.
        assert!(bounds.min.x > 5.9375 && bounds.min.x < 6.0);
        assert_eq!(
            fixture.world.get_block(&BlockPos::new(5, 2, 4)),
            &Block::HONEY_BLOCK
        );
        assert!(fixture.player().awaiting_teleport.lock().unwrap().is_none());
        assert!(!fixture.player().client.closed());
        fixture.assert_landed(20.0, 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_damage_callback_cannot_commit_to_retired_session() {
    let fixture = MovementTest::new(&Block::STONE, false);
    let profile = fixture.player().gameprofile.clone();
    let (client, _packets) = captured_client(profile.clone());
    let replacement = Arc::new(Player::new(
        Arc::new(ClientPlatform::Java(client)),
        profile,
        PlayerConfig::default(),
        &fixture.world,
        GameMode::Survival,
    ));
    replacement.set_client_loaded(true);
    replacement.set_health(18.0);
    replacement.living_entity.fall_distance.store(7.0);
    let current = replacement.clone();
    let world = fixture.world.clone();
    let callbacks = Arc::new(AtomicUsize::new(0));
    let seen = callbacks.clone();
    on_damage(&fixture.server, move |event| {
        assert_eq!(event.damage, 1.0);
        assert_eq!(seen.fetch_add(1, Relaxed), 0);
        world.players.store(Arc::new(vec![current.clone()]));
    });

    fixture.position(1.6, false, false);
    fixture.position(1.0, true, false);
    let published = fixture
        .world
        .get_player_by_uuid(replacement.gameprofile.id)
        .unwrap();
    assert!(Arc::ptr_eq(&published, &replacement));
    assert!(!Arc::ptr_eq(&published, fixture.player()));
    assert!(replacement.has_client_loaded());
    assert!(!replacement.client.closed());
    assert_eq!(callbacks.load(Relaxed), 1);
    assert_eq!(fixture.player().living_entity.health.load(), 20.0);
    assert_eq!(replacement.living_entity.health.load(), 18.0);
    assert_eq!(replacement.living_entity.fall_distance.load(), 7.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_mace_particles_keep_server_player_override_order() {
    use pumpkin_data::{packet::clientbound::play::LEVEL_PARTICLES, particle::Particle};

    for mode in 0..3 {
        let mut fixture = MovementTest::new(&Block::STONE, false);
        fixture
            .player()
            .spawn_extra_particles_on_fall
            .store(true, Relaxed);
        if mode != 1 {
            fixture.position(1.6, false, false);
        }
        if mode == 2 {
            fixture
                .world
                .level
                .loaded_chunks
                .get(&Vector2::new(0, 0))
                .unwrap()
                .set_block_absolute_y(4, 1, 4, Block::WATER.default_state.id);
        }
        fixture.position(1.0, true, false);
        fixture.assert_landed(
            if mode == 2 { 20.0 } else { 19.0 },
            if mode == 2 { 0 } else { 400 },
        );
        let counts: Vec<_> = fixture
            .client
            .take_packets()
            .into_iter()
            .filter_map(|packet| {
                let mut bytes = packet.as_ref();
                (bytes.get_var_int().unwrap().0 == LEVEL_PARTICLES.0).then(|| {
                    // 26.3 places particle options first, then XYZ speed and a VarInt count.
                    assert_eq!(
                        bytes.get_var_int().unwrap().0,
                        Particle::Block.to_id() as i32
                    );
                    assert_eq!(
                        bytes.get_var_int().unwrap().0,
                        i32::from(Block::STONE.default_state.id.as_u16())
                    );
                    assert!(!bytes.get_bool().unwrap());
                    assert!(!bytes.get_bool().unwrap());
                    for coordinate in [4.5, 1.0, 4.5] {
                        assert_eq!(bytes.get_f64_be().unwrap(), coordinate);
                    }
                    for _ in 0..3 {
                        assert_eq!(bytes.get_f32_be().unwrap(), 0.3);
                    }
                    for _ in 0..3 {
                        assert_eq!(bytes.get_f32_be().unwrap(), 0.15);
                    }
                    let count = bytes.get_var_int().unwrap().0;
                    assert_eq!(bytes.get_var_int().unwrap().0, 0);
                    assert!(bytes.is_empty());
                    count
                })
            })
            .collect();
        assert_eq!(counts, if mode == 1 { Vec::new() } else { vec![170] });
        assert_eq!(
            fixture.player().spawn_extra_particles_on_fall.load(Relaxed),
            mode == 1
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g09_old_save_fall_distance_and_runtime_defaults_survive_loading() {
    for (tag, distance) in [
        (Some("FallDistance"), 4.0),
        (Some("fall_distance"), 4.0),
        (None, 0.0),
        (Some("FallDistance"), -3.0),
    ] {
        let fixture = MovementTest::new(&Block::STONE, false);
        fixture.position(1.0, false, false);
        // Vanilla and older fork saves have no runtime landing reservation fields.
        let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
        nbt.put_float("Health", 20.0);
        if let Some(tag) = tag {
            nbt.put_float(tag, distance);
        }
        fixture.player().living_entity.read_living_nbt_non_mut(&nbt);
        assert_eq!(
            fixture.player().living_entity.fall_distance.load(),
            distance
        );
        fixture.ground(false);
        fixture.assert_landed(
            if distance > 0.0 { 19.0 } else { 20.0 },
            if distance > 0.0 { 400 } else { 0 },
        );
    }
}
