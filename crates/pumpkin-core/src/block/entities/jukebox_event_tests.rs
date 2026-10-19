use super::*;
use crate::block::{
    BlockBehaviour, BlockHitResult, GetComparatorOutputArgs, GetRedstonePowerArgs, NormalUseArgs,
    UseWithItemArgs, blocks::jukebox::JukeboxBlock,
};

use crate::{
    block::entities::block_entity_from_nbt,
    entity::{death_test_world::DeathTestWorld, player::Player},
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::world::generic_game::GenericGameEvent,
    },
    server::Server,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    Block, BlockDirection, biome::Biome, block_properties::JukeboxLikeProperties,
    data_component_impl::JukeboxPlayableImpl, game_event::GameEvent, item::Item,
    jukebox_song::JukeboxSong, packet::CURRENT_MC_VERSION,
};
use pumpkin_nbt::{Nbt, compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{VarInt, java::client::play::CBlockEntityData};
use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::{cylindrical_chunk_iterator::Cylindrical, world::BlockFlags};
use std::{num::NonZeroU8, sync::atomic::AtomicUsize};

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
    for _ in 0..22 {
        jukebox.tick(&world);
    }
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
    player.inventory().set_held_item(disc.clone());
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

fn jukebox_weak_power(world: &Arc<World>, position: &BlockPos) -> u8 {
    JukeboxBlock.get_weak_redstone_power(GetRedstonePowerArgs {
        world,
        block: &Block::JUKEBOX,
        state: world.get_block_state(position),
        position,
        direction: BlockDirection::Up,
    })
}

fn jukebox_comparator_output(world: &Arc<World>, position: &BlockPos) -> Option<u8> {
    JukeboxBlock.get_comparator_output(GetComparatorOutputArgs {
        world,
        block: &Block::JUKEBOX,
        state: world.get_block_state(position),
        position,
        direction: BlockDirection::Up,
    })
}

fn jukebox_wire_power(world: &World, position: &BlockPos) -> u8 {
    pumpkin_data::block_properties::RedstoneWireLikeProperties::from_state_id(
        world.get_block_state_id(position),
    )
    .power
}

fn add_test_jukebox(world: &Arc<World>, position: BlockPos) -> Arc<JukeboxBlockEntity> {
    world.set_block_state(
        &position,
        Block::JUKEBOX.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let jukebox = Arc::new(JukeboxBlockEntity::new(position));
    world.add_block_entity(jukebox.clone());
    jukebox
}

fn save_block_entity(block_entity: &dyn BlockEntity) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    block_entity.write_internal(&mut nbt);
    nbt
}

fn jukebox_nbt_at_elapsed_ticks(
    world: &Arc<World>,
    position: BlockPos,
    elapsed_ticks: u64,
) -> NbtCompound {
    let jukebox = add_test_jukebox(world, position);
    jukebox.set_record(ItemStack::new(1, &Item::MUSIC_DISC_CAT));
    jukebox.start_playing(JukeboxSong::Cat.length_in_ticks());
    let mut nbt = save_block_entity(jukebox.as_ref());
    nbt.put_long(TICKS_SINCE_SONG_STARTED_NBT_KEY, elapsed_ticks as i64);
    nbt
}

fn factory_reload_in_world(world: &Arc<World>, nbt: &NbtCompound) -> Arc<dyn BlockEntity> {
    let restored = block_entity_from_nbt(nbt).expect("jukebox NBT should load through the factory");
    let position = restored.get_position();
    world.add_block_entity(restored.clone());
    let installed = world
        .get_block_entity(&position)
        .expect("factory-loaded jukebox should be installed in the world");
    assert!(Arc::ptr_eq(&restored, &installed));
    installed
}

fn block_entity_packet_len(block_entity: &dyn BlockEntity) -> usize {
    let data = block_entity
        .chunk_data_nbt()
        .expect("jukeboxes have block entity data");
    let bytes = Nbt::from(data).write_unnamed();
    let packet = CBlockEntityData::new(
        block_entity.get_position(),
        VarInt(block_entity.get_id() as i32),
        bytes.as_ref().into(),
    );
    crate::net::java::JavaClient::serialize_packet_for_version(&packet, CURRENT_MC_VERSION)
        .expect("block entity packet should serialize")
        .len()
}

fn insert_disc_via_player_action(
    fixture: &DeathTestWorld,
    world: &Arc<World>,
    position: &BlockPos,
    player: &Arc<Player>,
    disc: &mut ItemStack,
) {
    player.inventory().set_held_item(disc.clone());
    let cursor = pumpkin_util::math::vector3::Vector3::new(0.5, 1.0, 0.5);
    let hit = BlockHitResult {
        face: &BlockDirection::Up,
        cursor_pos: &cursor,
    };
    assert!(matches!(
        JukeboxBlock.use_with_item(UseWithItemArgs {
            server: &fixture.server,
            world,
            block: &Block::JUKEBOX,
            position,
            player,
            hit: &hit,
            item_stack: disc,
            equipment_slot: &pumpkin_data::data_component_impl::EquipmentSlot::MAIN_HAND,
        }),
        crate::block::registry::BlockActionResult::Success
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g20_active_playback_survives_factory_reload() {
    let (fixture, jukebox, changes) = create_jukebox().await;
    let world = fixture.world();
    let position = jukebox.get_position();
    let wire_position = position.east();
    world.set_block_state(
        &wire_position.down(),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    world.set_block_state(
        &wire_position,
        Block::REDSTONE_WIRE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    assert_eq!(jukebox_wire_power(&world, &wire_position), 0);
    let player = fixture.player("JukeboxReload");
    player.watched_section.store(Cylindrical::new(
        Vector2::new(0, 0),
        NonZeroU8::new(2).expect("nonzero view distance"),
    ));
    let client = player
        .client
        .as_ref()
        .java()
        .expect("the fixture player uses Java");

    let mut disc = ItemStack::new(1, &Item::MUSIC_DISC_CAT);
    insert_disc_via_player_action(&fixture, &world, &position, &player, &mut disc);
    assert_eq!(changes.load(Ordering::Relaxed), 1);

    for _ in 0..20 {
        jukebox.tick(&world);
    }
    assert!(jukebox.is_playing());
    assert_eq!(jukebox_weak_power(&world, &position), 15);
    assert_eq!(jukebox_comparator_output(&world, &position), Some(2));
    let record = jukebox.get_record();
    let nbt = save_block_entity(jukebox.as_ref());
    assert_eq!(nbt.get_long("ticks_since_song_started"), Some(20));

    let restored = block_entity_from_nbt(&nbt).expect("saved jukebox should load through factory");
    let expected_packet_len = block_entity_packet_len(restored.as_ref());
    // Leave dust unpowered so the restored active record must propagate its output.
    world.set_block_state(
        &wire_position,
        Block::REDSTONE_WIRE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    assert_eq!(jukebox_wire_power(&world, &wire_position), 0);
    let packets_before_reload = client.pending_bytes.load(Ordering::Relaxed);
    world.add_block_entity(restored.clone());
    let packets_after_reload = client.pending_bytes.load(Ordering::Relaxed);
    assert_eq!(
        packets_after_reload - packets_before_reload,
        expected_packet_len,
        "loading sends the block entity update without replaying the jukebox start event"
    );
    assert_eq!(changes.load(Ordering::Relaxed), 1);

    let installed = world
        .get_block_entity(&position)
        .expect("factory-loaded jukebox should be installed");
    assert!(Arc::ptr_eq(&restored, &installed));
    let restored = installed
        .as_any()
        .downcast_ref::<JukeboxBlockEntity>()
        .expect("factory returns a jukebox block entity");
    assert!(restored.get_record().are_equal(&record));
    assert!(restored.is_playing());
    assert_eq!(jukebox_weak_power(&world, &position), 15);
    assert_eq!(jukebox_wire_power(&world, &wire_position), 15);
    assert_eq!(jukebox_comparator_output(&world, &position), Some(2));
    assert_eq!(
        save_block_entity(restored).get_long("ticks_since_song_started"),
        Some(20),
        "reload must preserve elapsed ticks instead of restarting at zero"
    );

    let finish_ticks = JukeboxSong::Cat.length_in_ticks() + 1;
    for _ in 0..finish_ticks {
        installed.tick(&world);
    }
    assert!(!restored.is_playing());
    assert_eq!(jukebox_weak_power(&world, &position), 0);
    assert_eq!(jukebox_wire_power(&world, &wire_position), 0);
    assert!(!restored.get_record().is_empty());
    assert_eq!(jukebox_comparator_output(&world, &position), Some(2));

    let stopped_nbt = save_block_entity(restored);
    assert_eq!(
        stopped_nbt.get_long(TICKS_SINCE_SONG_STARTED_NBT_KEY),
        None,
        "stopped playback omits its elapsed-tick tag"
    );
    let stopped_reload = factory_reload_in_world(&world, &stopped_nbt);
    let stopped_reload = stopped_reload
        .as_any()
        .downcast_ref::<JukeboxBlockEntity>()
        .expect("factory returns a jukebox block entity");
    assert!(!stopped_reload.is_playing());
    assert_eq!(jukebox_wire_power(&world, &wire_position), 0);
    assert_eq!(jukebox_weak_power(&world, &position), 0);
    assert_eq!(jukebox_comparator_output(&world, &position), Some(2));
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g20_stopped_finished_missing_and_invalid_song_controls() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));

    let ended_position = BlockPos::new(8, 64, 8);
    let ended = add_test_jukebox(&world, ended_position);
    ended.set_record(ItemStack::new(1, &Item::MUSIC_DISC_CAT));
    ended.start_playing(JukeboxSong::Cat.length_in_ticks());
    for _ in 0..=(JukeboxSong::Cat.length_in_ticks() + JUKEBOX_SONG_FINISH_PADDING_TICKS) {
        ended.tick(&world);
    }
    let ended_nbt = save_block_entity(ended.as_ref());
    assert_eq!(
        ended_nbt.get_long("ticks_since_song_started"),
        None,
        "completed playback does not retain an elapsed playback tag"
    );
    let ended_loaded = factory_reload_in_world(&world, &ended_nbt);
    let ended_loaded = ended_loaded
        .as_any()
        .downcast_ref::<JukeboxBlockEntity>()
        .expect("factory returns a jukebox block entity");
    assert!(!ended_loaded.is_playing());
    assert!(!ended_loaded.get_record().is_empty());
    assert_eq!(jukebox_weak_power(&world, &ended_position), 0);
    assert_eq!(jukebox_comparator_output(&world, &ended_position), Some(2));

    let stopped_position = BlockPos::new(9, 64, 8);
    let stopped = add_test_jukebox(&world, stopped_position);
    stopped.set_record(ItemStack::new(1, &Item::MUSIC_DISC_CAT));
    let stopped_nbt = save_block_entity(stopped.as_ref());
    assert_eq!(stopped_nbt.get_long("ticks_since_song_started"), None);
    let stopped_loaded = factory_reload_in_world(&world, &stopped_nbt);
    let stopped_loaded = stopped_loaded
        .as_any()
        .downcast_ref::<JukeboxBlockEntity>()
        .expect("factory returns a jukebox block entity");
    assert!(!stopped_loaded.is_playing());
    assert_eq!(jukebox_weak_power(&world, &stopped_position), 0);
    assert_eq!(
        jukebox_comparator_output(&world, &stopped_position),
        Some(2)
    );

    let empty_position = BlockPos::new(10, 64, 8);
    let empty = add_test_jukebox(&world, empty_position);
    let empty_nbt = save_block_entity(empty.as_ref());
    let empty_loaded = factory_reload_in_world(&world, &empty_nbt);
    let empty_loaded = empty_loaded
        .as_any()
        .downcast_ref::<JukeboxBlockEntity>()
        .expect("factory returns a jukebox block entity");
    assert!(empty_loaded.get_record().is_empty());
    assert!(!empty_loaded.is_playing());
    assert_eq!(jukebox_weak_power(&world, &empty_position), 0);
    assert_eq!(jukebox_comparator_output(&world, &empty_position), Some(0));

    let invalid_position = BlockPos::new(11, 64, 8);
    let invalid = add_test_jukebox(&world, invalid_position);
    invalid.set_record(ItemStack::new(1, &Item::MUSIC_DISC_CAT));
    invalid.start_playing(JukeboxSong::Cat.length_in_ticks());
    for _ in 0..20 {
        invalid.tick(&world);
    }
    let mut invalid_nbt = save_block_entity(invalid.as_ref());
    let mut record_nbt = invalid_nbt
        .get_compound("RecordItem")
        .expect("record should be serialized")
        .clone();
    let mut components = NbtCompound::new();
    let mut playable = NbtCompound::new();
    playable.put_string("song", "minecraft:not_a_song".to_owned());
    components.put("minecraft:jukebox_playable", NbtTag::Compound(playable));
    record_nbt.put("components", NbtTag::Compound(components));
    invalid_nbt.put("RecordItem", NbtTag::Compound(record_nbt));
    let invalid_loaded = factory_reload_in_world(&world, &invalid_nbt);
    let invalid_loaded = invalid_loaded
        .as_any()
        .downcast_ref::<JukeboxBlockEntity>()
        .expect("factory returns a jukebox block entity");
    assert!(!invalid_loaded.get_record().is_empty());
    assert_eq!(
        invalid_loaded
            .get_record()
            .get_data_component::<JukeboxPlayableImpl>()
            .map(|playable| playable.song),
        Some("")
    );
    assert!(!invalid_loaded.is_playing());
    assert_eq!(jukebox_weak_power(&world, &invalid_position), 0);
    assert_eq!(
        jukebox_comparator_output(&world, &invalid_position),
        Some(0)
    );

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g20_finish_padding_survives_factory_round_trip() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));

    for (index, elapsed_ticks, expected_playing) in
        [(0, 3700, true), (1, 3719, true), (2, 3720, false)]
    {
        let position = BlockPos::new(12 + index, 64, 8);
        let nbt = jukebox_nbt_at_elapsed_ticks(&world, position, elapsed_ticks);
        let loaded = factory_reload_in_world(&world, &nbt);
        let loaded = loaded
            .as_any()
            .downcast_ref::<JukeboxBlockEntity>()
            .expect("factory returns a jukebox block entity");

        assert_eq!(
            loaded.is_playing(),
            expected_playing,
            "elapsed={elapsed_ticks}"
        );
        assert!(!loaded.get_record().is_empty());
        assert_eq!(
            save_block_entity(loaded).get_long(TICKS_SINCE_SONG_STARTED_NBT_KEY),
            expected_playing.then_some(elapsed_ticks as i64),
            "the first save must preserve unfinished elapsed ticks and omit finished playback"
        );

        let saved = save_block_entity(loaded);
        let reloaded = factory_reload_in_world(&world, &saved);
        let reloaded = reloaded
            .as_any()
            .downcast_ref::<JukeboxBlockEntity>()
            .expect("factory returns a jukebox block entity");
        assert_eq!(
            reloaded.is_playing(),
            expected_playing,
            "elapsed={elapsed_ticks}"
        );
        assert!(!reloaded.get_record().is_empty());
        assert_eq!(
            jukebox_weak_power(&world, &position),
            if expected_playing { 15 } else { 0 }
        );
        assert_eq!(jukebox_comparator_output(&world, &position), Some(2));

        if elapsed_ticks == 3700 {
            for _ in 0..19 {
                reloaded.tick(&world);
            }
            assert!(reloaded.is_playing());
            assert_eq!(
                save_block_entity(reloaded).get_long(TICKS_SINCE_SONG_STARTED_NBT_KEY),
                Some(3719)
            );

            reloaded.tick(&world);
            assert!(reloaded.is_playing());
            assert_eq!(
                save_block_entity(reloaded).get_long(TICKS_SINCE_SONG_STARTED_NBT_KEY),
                Some(3720),
                "vanilla checks completion before incrementing elapsed ticks"
            );

            reloaded.tick(&world);
            assert!(!reloaded.is_playing());
            assert!(
                save_block_entity(reloaded)
                    .get_long(TICKS_SINCE_SONG_STARTED_NBT_KEY)
                    .is_none()
            );
            assert!(!reloaded.get_record().is_empty());
            assert_eq!(jukebox_weak_power(&world, &position), 0);
            assert_eq!(jukebox_comparator_output(&world, &position), Some(2));
        }
    }

    fixture.server.shutdown().await;
}
