use super::*;
use crate::block::entities::BlockEntity;
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::math::vector2::Vector2;
use std::time::Duration;

const BOUND: Duration = Duration::from_secs(5);

fn pay(player: &Player) -> Arc<dyn Inventory> {
    let payment = beacon_payment(player);
    payment.set_stack(0, ItemStack::new(1, &Item::EMERALD));
    payment
}

fn select_on_worker(
    player: Arc<Player>,
    primary: i32,
    secondary: i32,
) -> crossbeam::channel::Receiver<()> {
    let runtime = tokio::runtime::Handle::current();
    let (finished, wait_for_finish) = crossbeam::channel::bounded(1);
    drop(std::thread::spawn(move || {
        let _runtime = runtime.enter();
        player.client.java().unwrap().handle_set_beacon(
            &player,
            &SSetBeacon {
                primary_effect: Some(VarInt(primary)),
                secondary_effect: Some(VarInt(secondary)),
            },
        );
        finished.send(()).unwrap();
    }));
    wait_for_finish
}

async fn reload(fixture: &Fixture) -> Arc<dyn BlockEntity> {
    let pos = fixture.pos.chunk_position();
    let chunk = fixture.world.level.loaded_chunks.get(&pos).unwrap().clone();
    fixture
        .world
        .level
        .chunk_saver
        .save_chunks(&fixture.world.level.level_folder, vec![(pos, chunk)])
        .await
        .unwrap();
    fixture.world.block_entities.remove(&pos);
    fixture.world.level.loaded_chunks.remove(&pos);
    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    fixture
        .world
        .level
        .chunk_saver
        .fetch_chunks(&fixture.world.level.level_folder, &[pos], send)
        .await;
    let LoadedData::Loaded(chunk) = tokio::time::timeout(BOUND, receive.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("saved beacon chunk missing")
    };
    fixture.world.level.loaded_chunks.insert(pos, chunk);
    fixture.world.get_block_entity(&fixture.pos).unwrap()
}

fn second_viewer(fixture: &Fixture) -> TestPlayer {
    let second = TestPlayer::new(&fixture.world);
    second.client().set_player(second.player.clone());
    second
        .player
        .screen_handler_sync_handler
        .store_player(second.player.clone());
    second
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 2.5));
    fixture.world.players.store(Arc::new(vec![
        fixture.player.player.clone(),
        second.player.clone(),
    ]));
    fixture.use_block(&second, fixture.pos);
    second
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_overlapping_paid_selections_reject_older_capture_on_save_reload() {
    let fixture = Fixture::new(4);
    fixture.use_beacon();
    let second = second_viewer(&fixture);
    let first_payment = pay(&fixture.player.player);
    let second_payment = pay(&second.player);
    let (captured, wait_for_capture) = crossbeam::channel::bounded(1);
    let (resume, wait_for_resume) = crossbeam::channel::bounded(1);
    *fixture
        .world
        .persistence_test_hooks
        .update_capture
        .lock()
        .unwrap() = Some(Arc::new(move || {
        captured.send(()).unwrap();
        wait_for_resume.recv_timeout(BOUND).unwrap();
    }));
    let finished = select_on_worker(fixture.player.player.clone(), 0, 0);
    wait_for_capture.recv_timeout(BOUND).unwrap();
    set_beacon(&second, Some(2), Some(9));
    resume.send(()).unwrap();
    finished.recv_timeout(BOUND).unwrap();
    assert!(first_payment.get_stack(0).is_empty());
    assert!(second_payment.get_stack(0).is_empty());
    assert_eq!(fixture.beacon.primary_effect.load(Ordering::Relaxed), 2);
    let entity = reload(&fixture).await;
    let reloaded = entity.as_any().downcast_ref::<BeaconBlockEntity>().unwrap();
    assert_eq!(
        (
            reloaded.primary_effect.load(Ordering::Relaxed),
            reloaded.secondary_effect.load(Ordering::Relaxed)
        ),
        (2, 9)
    );
    second.client().close();
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_paid_selection_preserves_contended_persistent_fields_on_reload() {
    for name_field in [true, false] {
        let fixture = Fixture::new(4);
        *fixture.beacon.custom_name.lock().unwrap() = Some("Named beacon".into());
        *fixture.beacon.lock_key.lock().unwrap() = Some("saved lock".into());
        fixture.use_beacon();
        let payment = pay(&fixture.player.player);
        let beacon = fixture.beacon.clone();
        let (held, wait_for_lock) = crossbeam::channel::bounded(1);
        let (release, wait_for_release) = crossbeam::channel::bounded(1);
        let (unlocked, wait_for_unlock) = crossbeam::channel::bounded(1);
        drop(std::thread::spawn(move || {
            let guard = if name_field {
                beacon.custom_name.lock().unwrap()
            } else {
                beacon.lock_key.lock().unwrap()
            };
            held.send(()).unwrap();
            wait_for_release.recv_timeout(BOUND).unwrap();
            drop(guard);
            unlocked.send(()).unwrap();
        }));
        wait_for_lock.recv_timeout(BOUND).unwrap();
        let (captured, wait_for_capture) = crossbeam::channel::bounded(1);
        let (resume, wait_for_resume) = crossbeam::channel::bounded(1);
        *fixture
            .world
            .persistence_test_hooks
            .update_packet_capture
            .lock()
            .unwrap() = Some(Arc::new(move || {
            captured.send(()).unwrap();
            wait_for_resume.recv_timeout(BOUND).unwrap();
        }));
        let finished = select_on_worker(fixture.player.player.clone(), 2, 9);
        wait_for_capture.recv_timeout(BOUND).unwrap();
        release.send(()).unwrap();
        wait_for_unlock.recv_timeout(BOUND).unwrap();
        resume.send(()).unwrap();
        finished.recv_timeout(BOUND).unwrap();
        assert!(payment.get_stack(0).is_empty());
        let entity = reload(&fixture).await;
        let reloaded = entity.as_any().downcast_ref::<BeaconBlockEntity>().unwrap();
        assert_eq!(
            reloaded.custom_name.lock().unwrap().as_deref(),
            Some("Named beacon")
        );
        assert_eq!(
            reloaded.lock_key.lock().unwrap().as_deref(),
            Some("saved lock")
        );
        fixture.finish().await;
    }
}

fn power_selection_sounds(player: &mut TestPlayer) -> usize {
    player
        .take_packets()
        .into_iter()
        .filter(|packet| {
            let mut reader = packet.as_ref();
            if reader.get_var_int().unwrap().0 != pumpkin_data::packet::clientbound::play::SOUND.0
                || reader.get_var_int().unwrap().0
                    != i32::from(pumpkin_data::sound::Sound::BlockBeaconPowerSelect as u16) + 1
            {
                return false;
            }
            assert_eq!(
                reader.get_var_int().unwrap().0,
                pumpkin_data::sound::SoundCategory::Blocks as i32
            );
            assert_eq!(
                (
                    reader.get_i32_be().unwrap(),
                    reader.get_i32_be().unwrap(),
                    reader.get_i32_be().unwrap()
                ),
                (4, 516, 4)
            );
            assert_eq!(reader.get_f32_be().unwrap(), 1.0);
            assert_eq!(reader.get_f32_be().unwrap(), 1.0);
            true
        })
        .count()
}

fn refresh_beam(fixture: &Fixture) {
    for _ in 0..4 {
        fixture.beacon.tick(&fixture.world);
    }
    // This fixture supplies menu levels without constructing a pyramid.
    fixture.beacon.levels.store(4, Ordering::Relaxed);
}

fn set_stale_beacon_heightmap(fixture: &Fixture) {
    set_beacon_surface_heightmap(fixture, 62);
}

fn set_beacon_surface_heightmap(fixture: &Fixture, surface_y: i32) {
    let chunk = fixture
        .world
        .level
        .loaded_chunks
        .get(&fixture.pos.chunk_position())
        .unwrap();
    chunk.heightmap.lock().unwrap().set(
        pumpkin_world::chunk::ChunkHeightmapType::WorldSurface,
        0,
        0,
        surface_y,
        fixture.world.dimension.min_y,
    );
}

fn scan_beam_to_world_top(fixture: &Fixture) {
    let mut last_check_y = fixture.beacon.last_check_y.load(Ordering::Relaxed);
    if last_check_y >= fixture.world.get_top_y() {
        fixture.beacon.tick(&fixture.world);
        last_check_y = fixture.beacon.last_check_y.load(Ordering::Relaxed);
    }
    let scan_from_y = if last_check_y < fixture.pos.0.y {
        fixture.pos.0.y - 1
    } else {
        last_check_y
    };
    let blocks_to_scan = fixture.world.get_top_y() - scan_from_y;
    for _ in 0..(blocks_to_scan + 9) / 10 {
        fixture.beacon.tick(&fixture.world);
    }
    fixture.beacon.levels.store(4, Ordering::Relaxed);
}

fn assert_selection_sound(fixture: &mut Fixture, expected: usize) {
    let payment = pay(&fixture.player.player);
    fixture.player.take_packets();
    set_beacon(&fixture.player, Some(0), None);
    assert!(
        payment.get_stack(0).is_empty(),
        "paid selection with expected sound {expected} was not applied; levels {} active beam {} primary {}",
        fixture.beacon.levels.load(Ordering::Relaxed),
        fixture.beacon.has_active_beam(),
        fixture.beacon.primary_effect.load(Ordering::Relaxed)
    );
    assert_eq!(
        power_selection_sounds(&mut fixture.player),
        expected,
        "active beam: {}",
        fixture.beacon.has_active_beam()
    );
    set_beacon(&fixture.player, Some(2), None);
    assert_eq!(
        power_selection_sounds(&mut fixture.player),
        0,
        "unpaid selection played a sound"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_paid_power_selection_sound_tracks_beam_and_payment() {
    let mut fixture = Fixture::new(4);
    fixture.use_beacon();
    refresh_beam(&fixture);
    assert_selection_sound(&mut fixture, 1);
    let roof_position = BlockPos::new(0, 66, 0);
    fixture.world.set_block_state(
        &roof_position,
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    assert_selection_sound(&mut fixture, 1);
    refresh_beam(&fixture);
    assert_selection_sound(&mut fixture, 0);
    fixture.world.set_block_state(
        &roof_position,
        Block::AIR.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    assert_selection_sound(&mut fixture, 0);
    refresh_beam(&fixture);
    assert_selection_sound(&mut fixture, 1);
    for roof in [&Block::BEDROCK, &Block::RED_STAINED_GLASS] {
        fixture.world.set_block_state(
            &roof_position,
            roof.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        refresh_beam(&fixture);
        assert_selection_sound(&mut fixture, 1);
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_power_selection_sound_waits_for_complete_tick_scan() {
    let mut fixture = Fixture::new(4);
    // Older fork chunks can retain a surface below a player-built beacon.
    set_stale_beacon_heightmap(&fixture);
    fixture.beacon.tick(&fixture.world);
    assert!(!fixture.beacon.has_active_beam());

    scan_beam_to_world_top(&fixture);
    assert!(fixture.beacon.has_active_beam());
    fixture.use_beacon();
    assert_selection_sound(&mut fixture, 1);

    let roof_position = BlockPos::new(0, 73, 0);
    fixture.world.set_block_state(
        &roof_position,
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    set_stale_beacon_heightmap(&fixture);
    assert_eq!(fixture.world.get_top_block(Vector2::new(0, 0)), 62);
    fixture.beacon.tick(&fixture.world);
    assert!(!fixture.beacon.has_active_beam());
    fixture.beacon.levels.store(4, Ordering::Relaxed);
    assert_selection_sound(&mut fixture, 0);

    fixture.world.set_block_state(
        &roof_position,
        Block::AIR.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    set_stale_beacon_heightmap(&fixture);
    scan_beam_to_world_top(&fixture);
    assert!(fixture.beacon.has_active_beam());
    assert_selection_sound(&mut fixture, 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_stale_heightmap_one_below_beacon_still_scans_obstruction() {
    let fixture = Fixture::new(4);
    let roof_position = BlockPos::new(0, 66, 0);
    fixture.world.set_block_state(
        &roof_position,
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    set_beacon_surface_heightmap(&fixture, fixture.pos.0.y - 1);
    assert_eq!(
        fixture.world.get_top_block(Vector2::new(0, 0)),
        fixture.pos.0.y - 1
    );

    fixture.beacon.tick(&fixture.world);
    assert!(!fixture.beacon.has_active_beam());

    fixture.world.set_block_state(
        &roof_position,
        Block::AIR.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    set_beacon_surface_heightmap(&fixture, fixture.pos.0.y - 1);
    scan_beam_to_world_top(&fixture);
    assert!(fixture.beacon.has_active_beam());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_power_selection_sound_uses_beam_at_payment_commit() {
    let mut fixture = Fixture::new(4);
    fixture.use_beacon();
    refresh_beam(&fixture);
    pay(&fixture.player.player);
    fixture.player.take_packets();
    let (captured, wait_for_capture) = crossbeam::channel::bounded(1);
    let (resume, wait_for_resume) = crossbeam::channel::bounded(1);
    *fixture
        .world
        .persistence_test_hooks
        .update_capture
        .lock()
        .unwrap() = Some(Arc::new(move || {
        captured.send(()).unwrap();
        wait_for_resume.recv_timeout(BOUND).unwrap();
    }));
    let finished = select_on_worker(fixture.player.player.clone(), 0, 0);
    wait_for_capture.recv_timeout(BOUND).unwrap();
    fixture.world.set_block_state(
        &BlockPos::new(0, 66, 0),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    refresh_beam(&fixture);
    resume.send(()).unwrap();
    finished.recv_timeout(BOUND).unwrap();
    assert_eq!(power_selection_sounds(&mut fixture.player), 1);
    fixture.finish().await;
}
