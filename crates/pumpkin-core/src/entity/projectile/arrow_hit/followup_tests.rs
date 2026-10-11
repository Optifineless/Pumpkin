#![expect(
    clippy::unwrap_used,
    reason = "Motion regression fixtures must be valid"
)]

use super::{playtest_tests::motions, *};
use crate::{
    entity::{
        Entity,
        living::damage_transaction::test_hooks::{self, Point},
    },
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{Block, Enchantment, item::Item, item_stack::ItemStack};
use pumpkin_protocol::java::server::play::SPlayerPosition;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::chunk::ChunkData;

fn arrow(world: &Arc<crate::world::World>, position: Vector3<f64>, punch: u16) -> Arc<ArrowEntity> {
    let arrow = Arc::new(ArrowEntity::new(
        Entity::new(world.clone(), position, &EntityType::ARROW),
        None,
    ));
    let mut bow = ItemStack::new(1, &Item::BOW);
    if punch > 0 {
        bow.add_enchantment(&Enchantment::PUNCH, punch);
    }
    *arrow.weapon.write().unwrap() = Some(bow);
    arrow.entity.velocity.store(Vector3::new(0.0, 0.0, -1.6));
    arrow
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_successive_arrows_preserve_current_falling_y_after_accepted_movement() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let chunk = ChunkData::empty_sync(0, 0);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    player.get_entity().set_pos(Vector3::new(4.5, 100.0, 4.5));
    player.get_entity().on_ground.store(true, Ordering::Relaxed);
    arrow(&world, player.position() + Vector3::new(0.0, 0.5, 1.0), 0)
        .hit_entity(&(player.clone() as Arc<dyn EntityBase>), player.position());
    world.entity_tracker.update_all(&world);
    assert!((motions(&mut fixture, player.entity_id())[0].y - 0.4).abs() < 0.000_04);
    // Independent Java float evaluation of 12 travelInAir ticks from Y=0.4.
    for _ in 0..12 {
        fixture.client().handle_position(
            &player,
            &server,
            &SPlayerPosition {
                position: player.position() + Vector3::new(0.01, -0.01, 0.0),
                collision: 0,
            },
        );
        let accepted = player.position();
        player.living_entity.tick(player.as_ref(), &server);
        assert_eq!(player.position(), accepted);
    }
    let falling_y = -0.530_023_782_967_426_1;
    assert!((player.get_entity().velocity.load().y - falling_y).abs() < 1e-7);
    for punch in [0, 2] {
        player
            .living_entity
            .hurt_cooldown
            .store(0, Ordering::Relaxed);
        let current_y = player.get_entity().velocity.load().y;
        arrow(
            &world,
            player.position() + Vector3::new(0.0, 0.5, 1.0),
            punch,
        )
        .hit_entity(&(player.clone() as Arc<dyn EntityBase>), player.position());
        world.entity_tracker.update_all(&world);
        let packets = motions(&mut fixture, player.entity_id());
        assert_eq!(packets.len(), 1);
        assert!((packets[0].y - (current_y + if punch > 0 { 0.1 } else { 0.0 })).abs() < 0.000_04);
        player.living_entity.tick(player.as_ref(), &server);
    }
    // An accepted landing consumes downward movement before gravity, without resetting velocity.
    fixture.client().handle_position(
        &player,
        &server,
        &SPlayerPosition {
            position: player.position(),
            collision: 1,
        },
    );
    player.living_entity.tick(player.as_ref(), &server);
    assert!((player.get_entity().velocity.load().y + 0.078_400_001_525_878_9).abs() < 1e-7);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_player_motion_clips_ceiling_and_landing_without_moving_position() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(4, 63, 4, Block::STONE.default_state.id);
    chunk.set_block_absolute_y(4, 66, 4, Block::STONE.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let fixture = TestPlayer::new(&world);
    let player = fixture.player;
    player.get_entity().set_pos(Vector3::new(4.5, 64.0, 4.5));
    player.get_entity().on_ground.store(true, Ordering::Relaxed);
    arrow(&world, player.position() + Vector3::new(0.0, 0.5, 1.0), 0)
        .hit_entity(&(player.clone() as Arc<dyn EntityBase>), player.position());
    let accepted = player.position();
    // Entity.move discards the upward velocity at the ceiling, then travelInAir applies gravity.
    player.living_entity.tick(player.as_ref(), &server);
    assert!((player.get_entity().velocity.load().y + 0.078_400_001_525_878_9).abs() < 1e-7);
    assert_eq!(player.position(), accepted);
    // The next tick's downward motion collides with the floor and must not accumulate.
    player
        .get_entity()
        .on_ground
        .store(false, Ordering::Relaxed);
    player.living_entity.tick(player.as_ref(), &server);
    assert!((player.get_entity().velocity.load().y + 0.078_400_001_525_878_9).abs() < 1e-7);
    assert_eq!(player.position(), accepted);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_player_motion_keeps_horizontal_speed_below_vanilla_collision_epsilon() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(4, 63, 4, Block::STONE.default_state.id);
    for y in [64, 65] {
        chunk.set_block_absolute_y(5, y, 4, Block::STONE.default_state.id);
    }
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let player = TestPlayer::new(&world).player;
    let accepted = Vector3::new(4.600_005, 64.0, 4.5);
    player.get_entity().set_pos(accepted);
    player
        .get_entity()
        .on_ground
        .store(false, Ordering::Relaxed);
    player
        .get_entity()
        .velocity
        .store(Vector3::new(0.1, -0.0784, 0.0));
    // Entity.move / Mth.equal ignore this five-millionth horizontal clipping difference.
    player.living_entity.tick(player.as_ref(), &server);
    let motion = player.get_entity().velocity.load();
    assert!((motion.x - 0.091).abs() < 1e-7, "motion: {motion:?}");
    assert_eq!(player.position(), accepted);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_minecart_after_arrow_updates_observers_without_replaying_self_motion() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    let mut observer = TestPlayer::new(&world);
    world
        .players
        .store(Arc::new(vec![player.clone(), observer.player.clone()]));
    world
        .entity_tracker
        .get_tracked_entity(player.entity_id())
        .unwrap()
        .seen_by
        .insert(observer.player.gameprofile.id);
    world
        .entity_tracker
        .get_tracked_entity(player.entity_id())
        .unwrap()
        .last_section_pos
        .store(Vector3::new(0, 4, 0));
    player.get_entity().set_pos(Vector3::new(4.5, 64.0, 4.5));
    player.get_entity().on_ground.store(true, Ordering::Relaxed);
    arrow(&world, player.position() + Vector3::new(0.0, 0.5, 1.0), 0)
        .hit_entity(&(player.clone() as Arc<dyn EntityBase>), player.position());
    world.entity_tracker.update_all(&world);
    assert_eq!(motions(&mut fixture, player.entity_id()).len(), 1);
    assert_eq!(motions(&mut observer, player.entity_id()).len(), 1);
    let cart = crate::entity::r#type::from_type(
        &EntityType::MINECART,
        player.position() + Vector3::new(0.1, 0.0, 0.1),
        &world,
        uuid::Uuid::new_v4(),
    );
    for _ in 0..5 {
        cart.push(player.as_ref());
        world.entity_tracker.update_all(&world);
        assert!(motions(&mut fixture, player.entity_id()).is_empty());
        assert_eq!(motions(&mut observer, player.entity_id()).len(), 1);
    }
    player.set_velocity(Vector3::new(0.1, -0.2, 0.3));
    assert_eq!(motions(&mut fixture, player.entity_id()).len(), 1);
    assert_eq!(motions(&mut observer, player.entity_id()).len(), 1);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_world_tick_delivers_punch_once_after_projectile_followups() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(4, 63, 4, Block::STONE.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    player.get_entity().set_pos(Vector3::new(4.5, 64.0, 4.5));
    player.get_entity().on_ground.store(true, Ordering::Relaxed);
    let arrow = arrow(&world, Vector3::new(4.5, 64.5, 5.5), 2);
    world.add_entity_silent(arrow.clone());
    fixture.take_packets();
    let fixture = Arc::new(std::sync::Mutex::new(fixture));
    let capture = fixture.clone();
    let reached_tracking = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reached = reached_tracking.clone();
    let id = player.entity_id();
    test_hooks::install(move |point| {
        if point == Point::PlayerMotionFlush {
            assert!(
                motions(&mut capture.lock().unwrap(), id).is_empty(),
                "Punch sent intermediate motion before final tracking"
            );
            reached.store(true, Ordering::Relaxed);
        }
    });
    // World::tick must run the real arrow between player ticking and tracker synchronization.
    world.tick(&server);
    test_hooks::install(|_| {});
    assert!(reached_tracking.load(Ordering::Relaxed));
    assert!(
        arrow.entity.is_removed(),
        "arrow did not hit during entity ticking"
    );
    let packets = motions(&mut fixture.lock().unwrap(), player.entity_id());
    assert_eq!(packets.len(), 1);
    // Player travel has applied gravity before the grounded hit: (-0.0784 / 2 + 0.4) + Punch 0.1.
    assert!(
        (packets[0].y - 0.460_799_999_237_060_55).abs() < 0.000_1,
        "final Punch motion: {packets:?}"
    );
    fixture.lock().unwrap().take_packets();
    world.tick(&server);
    assert!(motions(&mut fixture.lock().unwrap(), player.entity_id()).is_empty());
    world.level.shutdown().await.unwrap();
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_entity_push_waits_for_motion_ready_restore() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let attacker = TestPlayer::new(&world).player;
    let victim = TestPlayer::new(&world).player;
    let zombie = crate::entity::r#type::from_type(
        &EntityType::ZOMBIE,
        victim.position() + Vector3::new(0.1, 0.0, 0.1),
        &world,
        uuid::Uuid::new_v4(),
    );
    let old = Vector3::new(0.2, -0.5, 0.3);
    let (start, started) = std::sync::mpsc::channel();
    let serialized = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let record = serialized.clone();
    let target = victim.clone();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            started.recv().unwrap();
            zombie.push(target.as_ref());
        });
        let target = victim.clone();
        test_hooks::install(move |point| {
            if point == Point::MotionReady {
                start.send(()).unwrap();
                record.store(
                    target.living_entity.wait_until_damage_contended(),
                    Ordering::Relaxed,
                );
            }
        });
        victim
            .get_entity()
            .velocity
            .store(Vector3::new(4.0, 0.4, 5.0));
        victim
            .get_entity()
            .hurt_marked
            .store(true, Ordering::Relaxed);
        attacker.send_hurt_motion(victim.as_ref(), old);
        test_hooks::install(|_| {});
    });
    assert!(
        serialized.load(Ordering::Relaxed),
        "push bypassed combat ownership"
    );
    let motion = victim.get_entity().velocity.load();
    assert_eq!(motion.y, old.y);
    assert!(motion.x < old.x && motion.z < old.z);
    assert!(victim.get_entity().velocity_dirty.load(Ordering::Relaxed));
    crate::server::fixture_lifecycle::finish().await;
}
