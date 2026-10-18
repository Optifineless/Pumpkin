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
async fn verification_cross_world_pearl_resets_after_transfer_completes() {
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

    let task_player = player.clone();
    test_player
        .with_outgoing_writer(async move {
            if let crate::net::ClientPlatform::Java(client) = task_player.client.as_ref() {
                client.await_tasks().await;
            }
        })
        .await;

    assert!(arrived_in_target_world.load(Ordering::Relaxed));
    assert_eq!(
        f32::from_bits(fall_distance_during_arrival.load(Ordering::Relaxed)),
        20.0
    );
    assert!(Arc::ptr_eq(&player.world(), &target_world));
    assert_eq!(player.living_entity.fall_distance.load(), 0.0);
    assert_eq!(player.living_entity.health.load(), 15.0);
}
