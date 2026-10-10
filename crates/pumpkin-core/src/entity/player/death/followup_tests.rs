#![expect(
    clippy::unwrap_used,
    reason = "Respawn regression fixtures must be valid"
)]

use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    plugin::{BoxFuture, EventHandler, EventPriority, player::player_respawn::PlayerRespawnEvent},
    server::{
        Server,
        combat_test_support::{server, world},
    },
};
use pumpkin_data::{Block, damage::DamageType, dimension::Dimension, item::Item};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use tokio::sync::Notify;

struct PausedRespawn {
    entered: Arc<Notify>,
    resume: Arc<Notify>,
}
impl EventHandler<PlayerRespawnEvent> for PausedRespawn {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerRespawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            assert_eq!(event.player.living_entity.health.load(), 20.0);
            self.entered.notify_one();
            self.resume.notified().await;
        })
    }
}

fn prepare_respawn(world: &Arc<crate::world::World>, player: &Player) {
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(12, 63, 12, Block::STONE.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    player.get_entity().set_pos(Vector3::new(4.5, 64.0, 4.5));
    player.set_respawn_point(
        Dimension::OVERWORLD,
        BlockPos::new(12, 64, 12),
        0.0,
        0.0,
        true,
    );
    player
        .screen_handler_sync_handler
        .store_player(world.get_player_by_id(player.entity_id()).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_respawn_callback_cannot_pick_up_old_death_drops_or_tick_or_take_damage() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    prepare_respawn(&world, &player);
    player
        .inventory
        .set_slot(0, ItemStack::new(1, &Item::DIAMOND));
    player
        .living_entity
        .damage(player.as_ref(), f32::MAX, DamageType::GENERIC_KILL);
    let old_life = player.living_entity.damage_lifecycle();
    let drops = world.entities.load_full();
    for drop in drops.iter().filter_map(|e| e.get_item_entity()) {
        drop.set_pickup_delay(0);
    }
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    server.plugin_manager.register::<PlayerRespawnEvent, _>(
        Arc::new(PausedRespawn {
            entered: entered.clone(),
            resume: resume.clone(),
        }),
        EventPriority::Normal,
        true,
    );
    fixture
        .collect_packets_during(async {
            tokio::join!(world.respawn_player(&player, false), async {
                entered.notified().await;
                assert_eq!(player.position(), Vector3::new(4.5, 64.0, 4.5));
                for drop in drops.iter() {
                    drop.on_player_collision(&player);
                }
                assert!(
                    player
                        .inventory
                        .main_inventory
                        .read()
                        .unwrap()
                        .iter()
                        .all(ItemStack::is_empty)
                );
                assert!(world.players.load().is_empty());
                let ticks = player.tick_counter.load(Relaxed);
                player.tick(&server);
                assert_eq!(player.tick_counter.load(Relaxed), ticks);
                assert!(!player.damage(player.as_ref(), f32::MAX, DamageType::GENERIC_KILL));
                resume.notify_one();
            });
        })
        .await;
    assert_eq!(player.position(), Vector3::new(12.5, 64.1, 12.5));
    assert_eq!(world.players.load().len(), 1);
    let ran_old_snapshot = AtomicBool::new(false);
    player.living_entity.for_published_life(old_life, || {
        ran_old_snapshot.store(true, Relaxed);
        for drop in drops.iter() {
            drop.on_player_collision(&player);
        }
    });
    assert!(!ran_old_snapshot.load(Relaxed));
    assert!(
        player
            .inventory
            .main_inventory
            .read()
            .unwrap()
            .iter()
            .all(ItemStack::is_empty)
    );
    assert_eq!(
        drops
            .iter()
            .filter_map(|e| e.get_item_entity())
            .map(|e| e.get_item_stack().lock().unwrap().item_count as u32)
            .sum::<u32>(),
        1
    );
    crate::server::fixture_lifecycle::finish().await;
}

struct RestoredRespawn;
impl EventHandler<PlayerRespawnEvent> for RestoredRespawn {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerRespawnEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(event.player.living_entity.health.load(), 20.0);
        assert!(!event.player.living_entity.dead.load(Relaxed));
        event.player.living_entity.set_health(7.0);
        event
            .player
            .inventory
            .set_slot(0, ItemStack::new(1, &Item::DIAMOND));
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_respawn_restores_before_callback_and_keeps_callback_changes() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    prepare_respawn(&world, &player);
    player.damage(player.as_ref(), f32::MAX, DamageType::GENERIC_KILL);
    server.plugin_manager.register::<PlayerRespawnEvent, _>(
        Arc::new(RestoredRespawn),
        EventPriority::Normal,
        true,
    );
    fixture
        .collect_packets_during(world.respawn_player(&player, false))
        .await;
    assert_eq!(player.living_entity.health.load(), 7.0);
    assert_eq!(
        player.inventory.main_inventory.read().unwrap()[0].item,
        &Item::DIAMOND
    );
    crate::server::fixture_lifecycle::finish().await;
}

struct RespawnDuringTick(Arc<Player>);
impl EventHandler<crate::plugin::entity::entity_damage::EntityDamageEvent> for RespawnDuringTick {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut crate::plugin::entity::entity_damage::EntityDamageEvent,
    ) -> BoxFuture<'a, ()> {
        assert!(self.0.living_entity.begin_respawn().is_some());
        self.0.living_entity.reset_state();
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_running_player_tick_stops_when_callback_begins_respawn() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world).player;
    server
        .plugin_manager
        .register::<crate::plugin::entity::entity_damage::EntityDamageEvent, _>(
            Arc::new(RespawnDuringTick(player.clone())),
            EventPriority::Normal,
            true,
        );
    player.get_entity().fire_ticks.store(20, Relaxed);
    // PlayerList.respawn replaces the Java object: its old tick cannot update the restored life.
    player.tick(&server);
    assert!(
        player.living_entity.is_respawning(),
        "fire damage never reached the callback"
    );
    assert_eq!(player.get_entity().fire_ticks.load(Relaxed), 0);
    assert_eq!(player.living_entity.hurt_cooldown.load(Relaxed), 20);
    assert_eq!(player.get_entity().velocity.load(), Vector3::default());
    assert_eq!(player.living_entity.health.load(), 20.0);
    crate::server::fixture_lifecycle::finish().await;
}
