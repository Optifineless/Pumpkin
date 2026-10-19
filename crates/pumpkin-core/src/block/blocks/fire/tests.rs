#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Focused gameplay fixtures fail loudly"
)]

use super::*;
use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::block::block_burn::BlockBurnEvent,
    },
    server::{Server, combat_test_support},
    world::World,
};
use bytes::Bytes;
use pumpkin_data::{
    BlockStateId,
    block_properties::{SlabType, WhiteWoolSlabLikeProperties},
    packet::CURRENT_MC_VERSION,
};
use pumpkin_protocol::{
    MultiVersionJavaPacket, ServerPacket,
    java::client::play::{CBlockUpdate, CMultiBlockUpdate},
    ser::NetworkReadExt,
};
use pumpkin_util::{
    GameMode, math::position::BlockPos, math::vector2::Vector2, math::vector3::Vector3,
};
use pumpkin_world::{tick::TickPriority, world::BlockFlags};
use rand::{RngExt, SeedableRng, rngs::StdRng};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

struct FireFixture {
    _directory: tempfile::TempDir,
    server: Arc<Server>,
    world: Arc<World>,
}

impl FireFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        world
            .forced_chunks
            .lock()
            .unwrap()
            .insert(Vector2::new(0, 0));
        world.level_info.rcu(|current| {
            let mut info = (**current).clone();
            info.game_rules.random_tick_speed = 0;
            info.game_rules.advance_weather = false;
            info
        });
        Self {
            _directory: directory,
            server,
            world,
        }
    }

    fn set_fire_radius(&self, radius: i64) {
        self.world.level_info.rcu(|current| {
            let mut info = (**current).clone();
            info.game_rules.fire_spread_radius_around_player = radius;
            info
        });
    }

    fn set_block(&self, pos: BlockPos, state_id: BlockStateId) {
        self.world.set_block_state(
            &pos,
            state_id,
            BlockFlags::FORCE_STATE | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
        );
    }

    fn set_fire(&self, pos: BlockPos, age: u8) -> BlockStateId {
        let mut properties = FireProperties::default(&Block::FIRE);
        properties.age = age;
        let state_id = properties.to_state_id(&Block::FIRE);
        self.set_block(pos, state_id);
        state_id
    }

    fn schedule_fire(&self, pos: BlockPos) {
        self.world
            .schedule_block_tick(&Block::FIRE, pos, 0, TickPriority::Normal);
    }

    fn tick(&self) {
        self.world.tick(&self.server);
    }

    fn tick_with_seed(&self, seed: u64) {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .start_handler(move |_| {
                TEST_FIRE_RANDOM.with(|rng| {
                    *rng.borrow_mut() = Some(StdRng::seed_from_u64(seed));
                });
            })
            .build()
            .unwrap();
        pool.install(|| self.world.tick(&self.server));
    }

    async fn shutdown(&self) {
        self.world.level.shutdown().await.unwrap();
    }
}

impl Drop for FireFixture {
    fn drop(&mut self) {
        self.world.level.cancel_token.cancel();
        self.world
            .level
            .shut_down_chunk_system
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.world.level.level_channel.notify();
    }
}

fn oak_slab(waterlogged: bool) -> BlockStateId {
    let mut properties = WhiteWoolSlabLikeProperties::default(&Block::OAK_SLAB);
    properties.r#type = SlabType::Bottom;
    properties.waterlogged = waterlogged;
    properties.to_state_id(&Block::OAK_SLAB)
}

fn seed_for_cancelled_and_control_burns(burn_odds: i32) -> u64 {
    (0..2_000_000)
        .find(|seed| {
            let mut rng = StdRng::seed_from_u64(*seed);
            let _delay: i32 = rng.random_range(0..10);
            let _age_increase = (rng.random_range(0..3) / 2) as u8;
            let cancelled_dry_target = rng.random_range(0..300) < burn_odds;
            let wet_target_burns_without_state_check = rng.random_range(0..300) < burn_odds;
            let _down_target = rng.random_range(0..300);
            let _up_target = rng.random_range(0..300);
            let dry_control_burns = rng.random_range(0..300) < burn_odds;
            cancelled_dry_target && wet_target_burns_without_state_check && dry_control_burns
        })
        .expect("bounded seeded RNG search finds dry and wet burn rolls")
}

fn seed_for_cancelled_burn(burn_odds: i32) -> u64 {
    (0..100_000)
        .find(|seed| {
            let mut rng = StdRng::seed_from_u64(*seed);
            let _delay = rng.random_range(0..10);
            let _age = rng.random_range(0..3);
            rng.random_range(0..300) < burn_odds
        })
        .expect("bounded seeded RNG search finds a cancelled burn roll")
}

fn seed_for_rain_extinction() -> u64 {
    (0..100_000)
        .find(|seed| {
            let mut rng = StdRng::seed_from_u64(*seed);
            let _delay = rng.random_range(0..10);
            rng.random::<f32>() < 0.2
        })
        .expect("bounded seeded RNG search finds a rain extinction roll")
}

fn capture_block_updates(packets: &[Bytes]) -> Vec<(BlockPos, u16)> {
    let mut updates = Vec::new();
    for packet in packets {
        let mut bytes = packet.as_ref();
        let id = bytes.get_var_int().unwrap().0;
        if id == CBlockUpdate::to_id(CURRENT_MC_VERSION) {
            let pos = bytes.get_block_pos(&CURRENT_MC_VERSION).unwrap();
            let state = bytes.get_var_int().unwrap().0 as u16;
            updates.push((pos, state));
        } else if id == CMultiBlockUpdate::to_id(CURRENT_MC_VERSION) {
            let packet = CMultiBlockUpdate::read(&mut bytes, &CURRENT_MC_VERSION).unwrap();
            updates.extend(
                packet
                    .updates
                    .into_iter()
                    .map(|(pos, state)| (pos, state.as_u16())),
            );
        }
    }
    updates
}

struct CancelBurnAt {
    position: BlockPos,
    calls: AtomicUsize,
}

impl EventHandler<BlockBurnEvent> for CancelBurnAt {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockBurnEvent,
    ) -> BoxFuture<'a, ()> {
        if event.block_pos == self.position {
            self.calls.fetch_add(1, Relaxed);
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduled_fire_b1019_g10_freezes_at_radius_zero_then_minus_one_control_extinguishes() {
    let fixture = FireFixture::new();
    let pos = BlockPos::new(8, 64, 8);
    fixture.set_block(pos.down(), Block::STONE.default_state.id);
    let age_15_fire = fixture.set_fire(pos, 15);
    fixture.set_fire_radius(0);
    fixture.schedule_fire(pos);
    fixture.tick();
    assert_eq!(
        fixture.world.get_block_state_id(&pos),
        age_15_fire,
        "radius 0 with no eligible players must preserve the identical fire state"
    );
    assert!(fixture.world.is_block_tick_scheduled(&pos, &Block::FIRE));

    fixture.set_fire_radius(-1);
    let next_tick = fixture.world.get_world_age() + 40;
    fixture.world.level_time.lock().unwrap().world_age = next_tick;
    fixture.world.level.set_game_time(next_tick);
    fixture.tick();
    assert_eq!(
        fixture.world.get_block_state_id(&pos),
        Block::AIR.default_state.id,
        "radius -1 must run the same age-15 fire on stone support and extinguish it"
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduled_fire_b1019_g10_includes_radius_boundary_and_excludes_spectators() {
    let fixture = FireFixture::new();
    let player = TestPlayer::new(&fixture.world);
    fixture.set_fire_radius(2);

    let boundary = BlockPos::new(4, 64, 4);
    fixture.set_fire(boundary, 15);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(6.5, 64.5, 4.5));
    fixture.schedule_fire(boundary);
    fixture.tick();
    assert_eq!(
        fixture.world.get_block_state_id(&boundary),
        Block::AIR.default_state.id,
        "a survival player exactly at the radius boundary is eligible"
    );

    let outside = BlockPos::new(8, 64, 8);
    let outside_fire = fixture.set_fire(outside, 15);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(10.5001, 64.5, 8.5));
    fixture.schedule_fire(outside);
    fixture.tick();
    assert_eq!(
        fixture.world.get_block_state_id(&outside),
        outside_fire,
        "a player just beyond the radius must not admit the source fire"
    );

    let spectator_source = BlockPos::new(12, 64, 12);
    let spectator_fire = fixture.set_fire(spectator_source, 15);
    player
        .player
        .get_entity()
        .set_pos(spectator_source.to_centered_f64());
    player.player.set_gamemode(GameMode::Spectator);
    fixture.schedule_fire(spectator_source);
    fixture.tick();
    assert_eq!(
        fixture.world.get_block_state_id(&spectator_source),
        spectator_fire,
        "spectators must not count as eligible fire-spread players"
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adjacent_burn_b1019_g10_preserves_waterlogged_wood_and_burns_dry_control() {
    let fixture = FireFixture::new();
    fixture.set_fire_radius(-1);
    let source = BlockPos::new(8, 64, 8);
    let dry = source.east();
    let wet = source.west();
    let dry_control = source.north();
    fixture.set_block(source.down(), Block::STONE.default_state.id);
    fixture.set_block(dry, oak_slab(false));
    fixture.set_block(wet, oak_slab(true));
    fixture.set_block(dry_control, oak_slab(false));
    fixture.set_fire(source, 0);

    let cancel = Arc::new(CancelBurnAt {
        position: dry,
        calls: AtomicUsize::new(0),
    });
    fixture.server.plugin_manager.register::<BlockBurnEvent, _>(
        cancel.clone(),
        EventPriority::Normal,
        true,
    );

    let burn_odds = i32::from(Block::OAK_SLAB.flammable.as_ref().unwrap().burn_chance);
    let seed = seed_for_cancelled_and_control_burns(burn_odds);
    fixture.schedule_fire(source);
    fixture.tick_with_seed(seed);

    assert_eq!(cancel.calls.load(Relaxed), 1);
    assert_eq!(
        fixture.world.get_block_state_id(&wet),
        oak_slab(true),
        "waterlogged wood must remain unchanged beside the same fire"
    );
    assert_ne!(
        fixture.world.get_block_state_id(&dry_control).to_block(),
        &Block::OAK_SLAB,
        "an admitted dry wood control must burn on the selected seeded tick"
    );
    assert_eq!(
        fixture.world.get_block_state_id(&dry),
        oak_slab(false),
        "the cancelled BlockBurnEvent must preserve its dry target"
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_block_burn_b1019_g10_keeps_dry_wood_unchanged() {
    let fixture = FireFixture::new();
    fixture.set_fire_radius(-1);
    let source = BlockPos::new(8, 64, 8);
    let target = source.east();
    fixture.set_block(source.down(), Block::STONE.default_state.id);
    fixture.set_block(target, oak_slab(false));
    fixture.set_fire(source, 0);
    let cancel = Arc::new(CancelBurnAt {
        position: target,
        calls: AtomicUsize::new(0),
    });
    fixture.server.plugin_manager.register::<BlockBurnEvent, _>(
        cancel.clone(),
        EventPriority::Normal,
        true,
    );

    let burn_odds = i32::from(Block::OAK_SLAB.flammable.as_ref().unwrap().burn_chance);
    fixture.schedule_fire(source);
    fixture.tick_with_seed(seed_for_cancelled_burn(burn_odds));
    assert_eq!(cancel.calls.load(Relaxed), 1);
    assert_eq!(fixture.world.get_block_state_id(&target), oak_slab(false));
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduled_extinction_b1019_g10_notifies_air_and_preserves_neighbor_blocks() {
    let fixture = FireFixture::new();
    fixture.set_fire_radius(-1);
    let mut observer = TestPlayer::new(&fixture.world);
    let source = BlockPos::new(8, 64, 8);
    let neighbor = source.east();
    let unaffected = source.north();
    fixture.set_block(neighbor, Block::STONE.default_state.id);
    fixture.set_block(unaffected, Block::STONE.default_state.id);
    fixture.set_fire(source, 0);
    observer.take_packets();
    fixture.schedule_fire(source);
    fixture.tick();
    fixture.world.flush_block_updates();

    assert_eq!(
        fixture.world.get_block_state_id(&source),
        Block::AIR.default_state.id
    );
    assert_eq!(
        fixture.world.get_block_state_id(&neighbor),
        Block::STONE.default_state.id
    );
    assert_eq!(
        fixture.world.get_block_state_id(&unaffected),
        Block::STONE.default_state.id
    );
    let updates = capture_block_updates(&observer.take_packets());
    assert!(updates.contains(&(source, Block::AIR.default_state.id.as_u16())));
    assert!(!updates.iter().any(|(pos, _)| *pos == neighbor));
    assert!(!updates.iter().any(|(pos, _)| *pos == unaffected));
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rain_extinction_b1019_g10_notifies_air_and_preserves_supported_neighbor_fire() {
    let fixture = FireFixture::new();
    fixture.set_fire_radius(-1);
    let mut observer = TestPlayer::new(&fixture.world);
    let source = BlockPos::new(8, 64, 8);
    let neighbor = source.east();
    fixture.set_block(source.down(), Block::STONE.default_state.id);
    fixture.set_block(neighbor.down(), Block::STONE.default_state.id);
    fixture.set_fire(source, 0);
    fixture.set_fire(neighbor, 0);
    fixture
        .world
        .weather
        .lock()
        .unwrap()
        .set_weather_parameters(&fixture.world, 0, 1000, true, false);
    let mut weather = fixture.world.weather.lock().unwrap();
    weather.rain_level = 1.0;
    weather.old_rain_level = 1.0;
    drop(weather);
    assert!(
        fixture.world.is_raining_at(&source),
        "rain fixture must expose the fire"
    );
    observer.take_packets();
    fixture.schedule_fire(source);
    fixture.tick_with_seed(seed_for_rain_extinction());
    fixture.world.flush_block_updates();

    assert_eq!(
        fixture.world.get_block_state_id(&source),
        Block::AIR.default_state.id
    );
    assert_eq!(fixture.world.get_block(&neighbor), &Block::FIRE);
    let updates = capture_block_updates(&observer.take_packets());
    assert!(updates.contains(&(source, Block::AIR.default_state.id.as_u16())));
    assert!(!updates.iter().any(|(pos, _)| *pos == neighbor));
    fixture.shutdown().await;
}
