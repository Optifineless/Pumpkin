use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::Server,
    server::combat_test_support::{server, world},
};
use pumpkin_data::Block;
use pumpkin_util::Difficulty;
use pumpkin_util::math::vector2::Vector2;
use std::sync::atomic::{AtomicBool, AtomicU32};
use tokio::sync::Notify;

struct EventObserverFn<F>(F);

impl<E, F> crate::plugin::EventHandler<E> for EventObserverFn<F>
where
    E: crate::plugin::Payload,
    F: Fn(&E) + Send + Sync,
{
    fn handle<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a E,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

fn register_observer<E: crate::plugin::Payload + Send + Sync + 'static>(
    server: &Server,
    action: impl Fn(&E) + Send + Sync + 'static,
) {
    server.plugin_manager.register::<E, _>(
        Arc::new(EventObserverFn(action)),
        crate::plugin::EventPriority::Normal,
        false,
    );
}

struct AsyncEventGate {
    entered: Notify,
    release: Notify,
    armed: AtomicBool,
}

impl AsyncEventGate {
    fn new() -> Self {
        Self {
            entered: Notify::new(),
            release: Notify::new(),
            armed: AtomicBool::new(true),
        }
    }

    async fn hold_once(&self) {
        if self.armed.swap(false, Ordering::AcqRel) {
            self.entered.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(30), self.release.notified())
                .await
                .expect("event gate was not released");
        }
    }

    async fn wait_until_entered(&self) {
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(30), self.entered.notified())
                .await
                .is_ok(),
            "event handler did not reach the gate"
        );
    }

    fn release(&self) {
        self.release.notify_one();
    }
}

impl<E: crate::plugin::Payload> crate::plugin::EventHandler<E> for AsyncEventGate {
    fn handle<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a E,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        Box::pin(self.hold_once())
    }
}

fn register_gate<E: crate::plugin::Payload + Send + Sync + 'static>(
    server: &Server,
) -> Arc<AsyncEventGate> {
    let gate = Arc::new(AsyncEventGate::new());
    server.plugin_manager.register::<E, _>(
        gate.clone(),
        crate::plugin::EventPriority::Normal,
        false,
    );
    gate
}

fn fire_test_pearl(
    player: &crate::entity::player::Player,
    world: &Arc<crate::world::World>,
    server: &Server,
) {
    world
        .level
        .loaded_chunks
        .get(&Vector2::new(0, 0))
        .expect("pearl destination chunk should be loaded")
        .set_block_absolute_y(8, 64, 8, Block::STONE.default_state.id);
    let pearl = ender_pearl::EnderPearlEntity::new_shot(
        Entity::new(world.clone(), Vector3::default(), &EntityType::ENDER_PEARL),
        player.get_entity(),
    );
    pearl.get_entity().set_pos(Vector3::new(5.5, 64.2, 8.5));
    pearl.get_entity().set_has_no_gravity(true);
    pearl
        .get_entity()
        .velocity
        .store(Vector3::new(2.0, 0.0, 0.0));

    pearl.tick(&pearl, server);
    assert!(!pearl.thrown.has_hit.load(Ordering::Relaxed));
    pearl.tick(&pearl, server);
    assert!(pearl.thrown.has_hit.load(Ordering::Relaxed));
}

async fn wait_for_player_tasks(
    test_player: &mut TestPlayer,
    player: Arc<crate::entity::player::Player>,
) {
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            test_player.with_outgoing_writer(async move {
                if let crate::net::ClientPlatform::Java(client) = player.client.as_ref() {
                    client.await_tasks().await;
                }
            }),
        )
        .await
        .is_ok(),
        "player tasks did not complete after the event gate opened"
    );
}

async fn wait_for_gate_with_writer(test_player: &mut TestPlayer, gate: &AsyncEventGate) {
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            test_player.with_outgoing_writer(gate.wait_until_entered()),
        )
        .await
        .is_ok(),
        "event handler did not reach the gate while the client writer ran"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_ender_pearl_cancelled_teleport_preserves_fall_and_position() {
    use crate::plugin::api::events::player::player_teleport::PlayerTeleportEvent;

    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    world.set_difficulty(Difficulty::Peaceful);
    let block = BlockPos::new(8, 64, 8);
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(
        block.0.x as usize,
        block.0.y,
        block.0.z as usize,
        Block::STONE.default_state.id,
    );
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);

    let test_player = TestPlayer::new(&world);
    let player = &test_player.player;
    let original_position = Vector3::new(1.0, 64.0, 1.0);
    player.get_entity().set_pos(original_position);
    player.living_entity.fall_distance.store(20.0);
    crate::entity::harvest_tests::register::<PlayerTeleportEvent>(&server, |event| {
        event.cancelled = true;
    });

    let pearl = ender_pearl::EnderPearlEntity::new_shot(
        Entity::new(world, Vector3::default(), &EntityType::ENDER_PEARL),
        player.get_entity(),
    );
    pearl.get_entity().set_pos(Vector3::new(5.5, 64.2, 8.5));
    pearl.get_entity().set_has_no_gravity(true);
    pearl
        .get_entity()
        .velocity
        .store(Vector3::new(2.0, 0.0, 0.0));

    pearl.tick(&pearl, &server);
    assert!(!pearl.thrown.has_hit.load(Ordering::Relaxed));
    pearl.tick(&pearl, &server);

    assert!(pearl.thrown.has_hit.load(Ordering::Relaxed));
    assert_eq!(player.get_entity().pos.load(), original_position);
    assert_eq!(player.living_entity.fall_distance.load(), 20.0);
    assert_eq!(player.living_entity.health.load(), 20.0);
}

#[tokio::test]
async fn verification_ender_pearl_clears_mace_impulse_before_landing() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    world.set_difficulty(Difficulty::Peaceful);
    let block = BlockPos::new(8, 66, 8);
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(
        block.0.x as usize,
        block.0.y,
        block.0.z as usize,
        Block::STONE.default_state.id,
    );
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);

    let test_player = TestPlayer::new(&world);
    let player = &test_player.player;
    player.get_entity().set_pos(Vector3::new(1.0, 64.0, 1.0));
    player.living_entity.protect_mace_landing();

    let pearl = ender_pearl::EnderPearlEntity::new_shot(
        Entity::new(world, Vector3::default(), &EntityType::ENDER_PEARL),
        player.get_entity(),
    );
    pearl.get_entity().set_pos(Vector3::new(5.5, 66.2, 8.5));
    pearl.get_entity().set_has_no_gravity(true);
    pearl
        .get_entity()
        .velocity
        .store(Vector3::new(2.0, 0.0, 0.0));

    pearl.tick(&pearl, &server);
    assert!(!pearl.thrown.has_hit.load(Ordering::Relaxed));
    let teleport_position = pearl.get_entity().pos.load();
    pearl.tick(&pearl, &server);

    assert!(pearl.thrown.has_hit.load(Ordering::Relaxed));
    assert_eq!(player.get_entity().pos.load(), teleport_position);
    assert_eq!(player.living_entity.health.load(), 15.0);
    player
        .living_entity
        .hurt_cooldown
        .store(0, Ordering::Relaxed);
    player
        .living_entity
        .fall(player.as_ref(), -4.0, false, false);
    player
        .get_entity()
        .set_pos(Vector3::new(teleport_position.x, 62.2, teleport_position.z));
    player.living_entity.fall(player.as_ref(), 0.0, true, false);
    assert!(player.living_entity.health.load() < 15.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_cross_world_pearl_effects_commit_before_changed_world_event() {
    use crate::plugin::api::events::player::player_changed_world::PlayerChangedWorldEvent;
    use pumpkin_data::dimension::Dimension;

    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let source_world = server.create_world("overworld".into(), Dimension::OVERWORLD);
    let target_world = server.create_world("nether".into(), Dimension::THE_NETHER);
    target_world.set_difficulty(Difficulty::Peaceful);
    let block = BlockPos::new(8, 64, 8);
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(
        block.0.x as usize,
        block.0.y,
        block.0.z as usize,
        Block::STONE.default_state.id,
    );
    target_world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), chunk);

    let mut test_player = TestPlayer::new(&source_world);
    let player = test_player.player.clone();
    player.get_entity().set_pos(Vector3::new(1.0, 64.0, 1.0));
    player.living_entity.fall_distance.store(20.0);
    let fall_distance_during_arrival = Arc::new(AtomicU32::new(f32::NAN.to_bits()));
    let arrived_in_target_world = Arc::new(AtomicBool::new(false));
    let observed_fall_distance = fall_distance_during_arrival.clone();
    let observed_target_world = arrived_in_target_world.clone();
    register_observer::<PlayerChangedWorldEvent>(&server, move |event| {
        observed_fall_distance.store(
            event.player.living_entity.fall_distance.load().to_bits(),
            Ordering::Relaxed,
        );
        observed_target_world.store(
            Arc::ptr_eq(&event.player.world(), &event.to_world),
            Ordering::Relaxed,
        );
    });

    let pearl = ender_pearl::EnderPearlEntity::new_shot(
        Entity::new(
            target_world.clone(),
            Vector3::default(),
            &EntityType::ENDER_PEARL,
        ),
        player.get_entity(),
    );
    pearl.get_entity().set_pos(Vector3::new(5.5, 64.2, 8.5));
    pearl.get_entity().set_has_no_gravity(true);
    pearl
        .get_entity()
        .velocity
        .store(Vector3::new(2.0, 0.0, 0.0));

    pearl.tick(&pearl, &server);
    assert!(!pearl.thrown.has_hit.load(Ordering::Relaxed));
    pearl.tick(&pearl, &server);
    assert!(pearl.thrown.has_hit.load(Ordering::Relaxed));

    wait_for_player_tasks(&mut test_player, player.clone()).await;

    assert!(arrived_in_target_world.load(Ordering::Relaxed));
    assert_eq!(
        f32::from_bits(fall_distance_during_arrival.load(Ordering::Relaxed)),
        0.0
    );
    assert!(Arc::ptr_eq(&player.world(), &target_world));
    assert_eq!(player.living_entity.fall_distance.load(), 0.0);
    assert_eq!(player.living_entity.health.load(), 15.0);
}

#[path = "ender_pearl_owner_review_tests.rs"]
mod owner_review_tests;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_pearl_losing_to_concurrent_world_transfer_has_no_effects() {
    use crate::plugin::api::events::player::player_change_world::PlayerChangeWorldEvent;
    use pumpkin_data::dimension::Dimension;

    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let source_world = server.create_world("overworld".into(), Dimension::OVERWORLD);
    let target_world = server.create_world("nether".into(), Dimension::THE_NETHER);
    target_world.level.loaded_chunks.insert(
        Vector2::new(0, 0),
        pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
    );

    let mut test_player = TestPlayer::new(&source_world);
    let player = test_player.player.clone();
    player.living_entity.fall_distance.store(20.0);
    let gate = register_gate::<PlayerChangeWorldEvent>(&server);
    fire_test_pearl(&player, &target_world, &server);
    gate.wait_until_entered().await;

    let competitor = player.clone();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            test_player.with_outgoing_writer(competitor.teleport_world(
                target_world.clone(),
                Vector3::new(2.0, 64.0, 2.0),
                Some(0.0),
                Some(0.0),
            )),
        )
        .await
        .is_ok(),
        "competing transfer did not complete"
    );
    assert!(Arc::ptr_eq(&player.world(), &target_world));

    gate.release();
    wait_for_player_tasks(&mut test_player, player.clone()).await;

    assert_eq!(player.living_entity.fall_distance.load(), 20.0);
    assert_eq!(player.living_entity.health.load(), 20.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_destination_landing_packet_runs_after_pearl_effects() {
    use crate::plugin::api::events::player::player_changed_world::PlayerChangedWorldEvent;
    use pumpkin_data::dimension::Dimension;
    use pumpkin_protocol::java::server::play::{FLAG_ON_GROUND, SConfirmTeleport, SPlayerPosition};

    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let source_world = server.create_world("overworld".into(), Dimension::OVERWORLD);
    let target_world = server.create_world("nether".into(), Dimension::THE_NETHER);
    target_world.set_difficulty(Difficulty::Peaceful);
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(8, 63, 8, Block::STONE.default_state.id);
    target_world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), chunk);

    let mut test_player = TestPlayer::new(&source_world);
    let player = test_player.player.clone();
    player.get_entity().set_pos(Vector3::new(1.0, 64.0, 1.0));
    player.living_entity.fall_distance.store(20.0);
    let gate = register_gate::<PlayerChangedWorldEvent>(&server);
    fire_test_pearl(&player, &target_world, &server);
    wait_for_gate_with_writer(&mut test_player, &gate).await;

    let Some(client) = player.client.as_ref().java() else {
        return;
    };
    crate::net::java::JavaClient::handle_player_loaded(&player);
    let (teleport_id, teleport_position) = player
        .awaiting_teleport
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .expect("world transfer should be awaiting the client's teleport confirmation");
    client.handle_confirm_teleport(
        &player,
        &SConfirmTeleport {
            teleport_id,
            position: teleport_position,
            yaw: 0.0,
            pitch: 0.0,
        },
    );
    client.handle_position(
        &player,
        &server,
        &SPlayerPosition {
            position: Vector3::new(8.5, 64.0, 8.5),
            collision: FLAG_ON_GROUND,
        },
    );

    assert_eq!(player.living_entity.health.load(), 15.0);
    assert_eq!(player.living_entity.fall_distance.load(), 0.0);

    gate.release();
    wait_for_player_tasks(&mut test_player, player).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_pending_pearl_does_not_damage_respawned_life() {
    use crate::plugin::api::events::player::player_change_world::PlayerChangeWorldEvent;
    use pumpkin_data::{damage::DamageType, dimension::Dimension};

    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let source_world = server.create_world("overworld".into(), Dimension::OVERWORLD);
    let target_world = server.create_world("nether".into(), Dimension::THE_NETHER);
    source_world.level.loaded_chunks.insert(
        Vector2::new(0, 0),
        pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
    );
    target_world.level.loaded_chunks.insert(
        Vector2::new(0, 0),
        pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
    );

    let mut test_player = TestPlayer::new(&source_world);
    let player = test_player.player.clone();
    let gate = register_gate::<PlayerChangeWorldEvent>(&server);
    fire_test_pearl(&player, &target_world, &server);
    gate.wait_until_entered().await;

    player.living_entity.set_health(0.0);
    player
        .living_entity
        .on_death(DamageType::GENERIC, None, None);
    let respawning_player = player.clone();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            test_player.with_outgoing_writer(respawning_player.respawn()),
        )
        .await
        .is_ok(),
        "player respawn did not complete"
    );
    assert_eq!(player.living_entity.health.load(), 20.0);
    player
        .living_entity
        .hurt_cooldown
        .store(0, Ordering::Relaxed);
    player.living_entity.fall_distance.store(7.0);

    gate.release();
    wait_for_player_tasks(&mut test_player, player.clone()).await;

    assert!(Arc::ptr_eq(&player.world(), &target_world));
    assert_eq!(player.living_entity.health.load(), 20.0);
    assert_eq!(player.living_entity.fall_distance.load(), 7.0);
    assert!(!player.has_client_loaded());
    // A fresh pearl must also preserve respawn loading immunity after the old transfer retires.
    player.get_entity().set_pos(Vector3::new(1.0, 64.0, 1.0));
    fire_test_pearl(&player, &target_world, &server);
    assert_eq!(player.living_entity.health.load(), 20.0);
    assert_eq!(player.living_entity.fall_distance.load(), 0.0);
}
