use super::*;
use crate::block::{
    BlockBehaviour, BlockHitResult, GetComparatorOutputArgs, GetRedstonePowerArgs, NormalUseArgs,
    UseWithItemArgs, blocks::jukebox::JukeboxBlock,
};

use crate::{
    entity::death_test_world::DeathTestWorld,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            block::block_physics::BlockPhysicsEvent, world::generic_game::GenericGameEvent,
        },
    },
    server::Server,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    Block, BlockDirection, biome::Biome, block_properties::JukeboxLikeProperties,
    game_event::GameEvent, item::Item, jukebox_song::JukeboxSong,
};
use pumpkin_world::world::BlockFlags;
use std::sync::atomic::AtomicUsize;

struct Changes(Arc<AtomicUsize>);
impl EventHandler<GenericGameEvent> for Changes {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key == GameEvent::BlockChange.name() {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        Box::pin(async {})
    }
}

fn assert_record(
    world: &World,
    position: &BlockPos,
    changes: &AtomicUsize,
    present: bool,
    count: usize,
) {
    assert_eq!(
        JukeboxLikeProperties::from_state_id(world.get_block_state_id(position)).has_record,
        present
    );
    assert_eq!(changes.load(Ordering::Relaxed), count);
}

async fn create_jukebox() -> (DeathTestWorld, Arc<JukeboxBlockEntity>, Arc<AtomicUsize>) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let changes = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(
            Arc::new(Changes(changes.clone())),
            EventPriority::Normal,
            true,
        );
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &position,
        Block::JUKEBOX.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let jukebox = Arc::new(JukeboxBlockEntity::new(position));
    world.add_block_entity(jukebox.clone());
    (fixture, jukebox, changes)
}

fn assert_outputs(world: &World, position: &BlockPos, power: u8, comparator: u8) {
    let state = world.get_block_state(position);
    assert_eq!(
        JukeboxBlock.get_weak_redstone_power(GetRedstonePowerArgs {
            world,
            block: &Block::JUKEBOX,
            state,
            position,
            direction: BlockDirection::East,
        }),
        power
    );
    assert_eq!(
        JukeboxBlock.get_comparator_output(GetComparatorOutputArgs {
            world,
            block: &Block::JUKEBOX,
            state,
            position,
            direction: BlockDirection::East,
        }),
        Some(comparator)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jukebox_update_slot_removal_stops_playback_and_updates_outputs() {
    struct PhysicsChanges(BlockPos, Arc<AtomicUsize>);
    impl EventHandler<BlockPhysicsEvent> for PhysicsChanges {
        fn handle_blocking<'a>(
            &'a self,
            _: &'a Arc<Server>,
            event: &'a mut BlockPhysicsEvent,
        ) -> BoxFuture<'a, ()> {
            if event.changed_pos == self.0 {
                self.1.fetch_add(1, Ordering::Relaxed);
            }
            Box::pin(async {})
        }
    }
    let (fixture, jukebox, changes) = create_jukebox().await;
    let world = fixture.world();
    let position = jukebox.get_position();
    jukebox.set_record(ItemStack::new(1, &Item::MUSIC_DISC_13));
    jukebox.start_playing(JukeboxSong::Id13.length_in_ticks());
    assert_outputs(&world, &position, 15, 1);
    jukebox.clear_comparator_dirty();
    let neighbors = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<BlockPhysicsEvent, _>(
            Arc::new(PhysicsChanges(position, neighbors.clone())),
            EventPriority::Normal,
            true,
        );
    jukebox.update_slot(0, &mut |record| {
        let _ = record.split(1);
    });
    assert!(!jukebox.is_playing(), "removed disc kept playing");
    assert_outputs(&world, &position, 0, 0);
    assert!(jukebox.is_comparator_dirty());
    assert!(neighbors.load(Ordering::Relaxed) > 0);
    assert_record(&world, &position, &changes, false, 2);
    fixture.server.shutdown().await;
}

#[test]
fn jukebox_update_slot_insertion_starts_playback_and_non_record_stops() {
    let jukebox = JukeboxBlockEntity::new(BlockPos::new(0, 0, 0));
    jukebox.update_slot(0, &mut |record| {
        *record = ItemStack::new(1, &Item::MUSIC_DISC_13);
    });
    assert!(jukebox.is_playing(), "inserted disc did not start playing");
    jukebox.ticks_since_song_started.store(1, Ordering::Relaxed);
    jukebox.clear_comparator_dirty();
    jukebox.update_slot(0, &mut |_| {});
    assert_eq!(jukebox.ticks_since_song_started.load(Ordering::Relaxed), 1);
    assert!(!jukebox.is_comparator_dirty());
    jukebox.update_slot(0, &mut |record| *record = ItemStack::new(1, &Item::STONE));
    assert!(!jukebox.is_playing());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jukebox_record_mutations_emit_one_block_change() {
    let (fixture, jukebox, changes) = create_jukebox().await;
    let world = fixture.world();
    let position = jukebox.get_position();
    let disc = ItemStack::new(1, &Item::MUSIC_DISC_13);
    jukebox.set_record(disc.clone());
    assert_record(&world, &position, &changes, true, 1);
    jukebox.set_record(disc.clone());
    assert_record(&world, &position, &changes, true, 1);
    jukebox.start_playing(1);
    jukebox.tick(&world);
    jukebox.tick(&world);
    assert!(!jukebox.is_playing());
    assert!(!jukebox.get_record().is_empty());
    assert_record(&world, &position, &changes, true, 1);
    assert!(jukebox.clear_record().are_equal(&disc));
    assert_record(&world, &position, &changes, false, 2);
    jukebox.set_stack(0, disc.clone());
    assert_record(&world, &position, &changes, true, 3);
    assert!(jukebox.remove_stack_specific(0, 1).are_equal(&disc));
    assert_record(&world, &position, &changes, false, 4);
    jukebox.set_stack(0, disc.clone());
    assert!(jukebox.remove_stack(0).are_equal(&disc));
    assert_record(&world, &position, &changes, false, 6);
    jukebox.set_stack(0, disc);
    jukebox.clear();
    assert_record(&world, &position, &changes, false, 8);
    jukebox.clear();
    assert_record(&world, &position, &changes, false, 8);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jukebox_player_actions_emit_one_block_change() {
    let (fixture, jukebox, changes) = create_jukebox().await;
    let world = fixture.world();
    let position = jukebox.get_position();
    let player = fixture.player("JukeboxEvents");
    let mut disc = ItemStack::new(2, &Item::MUSIC_DISC_13);
    let cursor = pumpkin_util::math::vector3::Vector3::new(0.5, 1.0, 0.5);
    let hit = BlockHitResult {
        face: &pumpkin_data::BlockDirection::Up,
        cursor_pos: &cursor,
    };
    JukeboxBlock.use_with_item(UseWithItemArgs {
        server: &fixture.server,
        world: &world,
        block: &Block::JUKEBOX,
        position: &position,
        player: &player,
        hit: &hit,
        item_stack: &mut disc,
        equipment_slot: &pumpkin_data::data_component_impl::EquipmentSlot::MAIN_HAND,
    });
    assert_record(&world, &position, &changes, true, 1);
    assert_eq!(disc.item_count, 1);
    JukeboxBlock.normal_use(NormalUseArgs {
        server: &fixture.server,
        world: &world,
        block: &Block::JUKEBOX,
        position: &position,
        player: &player,
        hit: &hit,
    });
    assert_record(&world, &position, &changes, false, 2);
    fixture.server.shutdown().await;
}

async fn paused_notification(removes_block: bool) {
    use std::sync::Barrier;
    let (fixture, jukebox, changes) = create_jukebox().await;
    let world = fixture.world();
    let position = jukebox.get_position();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let first = Arc::new(AtomicBool::new(true));
    *jukebox.notification_pause.lock().unwrap() = Some(Arc::new({
        let entered = entered.clone();
        let release = release.clone();
        move || {
            if first.swap(false, Ordering::Relaxed) {
                entered.wait();
                release.wait();
            }
        }
    }));
    std::thread::scope(|scope| {
        let setter = scope.spawn(|| jukebox.set_record(ItemStack::new(1, &Item::MUSIC_DISC_13)));
        entered.wait();
        if removes_block {
            assert!(
                world
                    .break_block(&position, None, BlockFlags::NOTIFY_ALL)
                    .is_some()
            );
        } else {
            jukebox.clear_record();
        }
        release.wait();
        setter.join().unwrap();
    });
    if removes_block {
        assert!(
            world.get_block_state(&position).is_air(),
            "stale record update resurrected jukebox"
        );
        assert!(world.get_block_entity(&position).is_none());
    } else {
        assert!(jukebox.get_record().is_empty());
        assert_record(&world, &position, &changes, false, 1);
    }
    assert!(
        !jukebox.is_playing(),
        "stale record notification restarted playback"
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jukebox_notification_cannot_resurrect_concurrently_broken_block() {
    paused_notification(true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jukebox_stale_notification_cannot_overwrite_newer_record() {
    paused_notification(false).await;
}
