use super::World;
use crate::{
    block::{
        BlockHitResult, NormalUseArgs,
        entities::{BlockEntity, chest::ChestBlockEntity},
    },
    entity::{Entity, EntityBase, living::LivingEntity},
    net::java::combat_test_support::TestPlayer,
    server::{
        Server,
        combat_test_support::{publish_empty_chunk, server, world},
    },
};
use pumpkin_data::{Block, BlockDirection, entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{
    VarInt,
    codec::item_stack_seralizer::OptionalItemStackHash,
    java::server::play::{SClickSlot, SlotActionType},
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::io::{FileIO, LoadedData};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    mpsc,
};

fn use_chest(
    server: &Arc<Server>,
    world: &Arc<World>,
    player: &Arc<crate::entity::player::Player>,
    pos: BlockPos,
) {
    let hit = BlockHitResult {
        face: &BlockDirection::Up,
        cursor_pos: &Vector3::new(0.5, 0.5, 0.5),
    };
    world
        .block_registry
        .get_pumpkin_block(Block::CHEST.id)
        .unwrap()
        .normal_use(NormalUseArgs {
            server,
            world,
            block: &Block::CHEST,
            position: &pos,
            player,
            hit: &hit,
        });
}

fn open_chest(
    server: &Arc<Server>,
    world: &Arc<World>,
    player: &Arc<crate::entity::player::Player>,
    pos: BlockPos,
) -> Arc<dyn Inventory> {
    use_chest(server, world, player, pos);
    let screen = player.current_screen_handler.lock().unwrap().clone();
    screen.lock().unwrap().get_behaviour().slots[0].get_inventory()
}

fn pending_chest(world: &World, pos: BlockPos) -> Arc<pumpkin_world::chunk::ChunkData> {
    publish_empty_chunk(world, pos.chunk_position());
    let chunk = world
        .level
        .loaded_chunks
        .get(&pos.chunk_position())
        .unwrap()
        .clone();
    chunk.set_block_absolute_y(8, 64, 8, Block::CHEST.default_state.id);
    // Both vanilla and older fork saves use this container NBT schema.
    let chest = ChestBlockEntity::new(pos);
    chest.set_stack(0, ItemStack::new(5, &Item::DIAMOND));
    let mut nbt = NbtCompound::new();
    chest.write_internal(&mut nbt);
    chunk
        .pending_block_entities
        .lock()
        .unwrap()
        .insert(pos, nbt);
    chunk
}

async fn unload_and_reload(
    world: &Arc<World>,
    pos: BlockPos,
    chunk: Arc<pumpkin_world::chunk::ChunkData>,
) {
    world
        .remove_entities_in_chunks([pos.chunk_position()])
        .await;
    world.tick(&world.server.upgrade().unwrap());
    world
        .level
        .chunk_saver
        .save_chunks(
            &world.level.level_folder,
            vec![(pos.chunk_position(), chunk)],
        )
        .await
        .unwrap();
    world.level.loaded_chunks.remove(&pos.chunk_position());
    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    world
        .level
        .chunk_saver
        .fetch_chunks(&world.level.level_folder, &[pos.chunk_position()], send)
        .await;
    let LoadedData::Loaded(reloaded) = receive.recv().await.unwrap() else {
        panic!("saved chest chunk missing")
    };
    world
        .level
        .loaded_chunks
        .insert(pos.chunk_position(), reloaded);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lazy_chest_publication_keeps_one_inventory_through_click_unload_and_reload() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let pos = BlockPos::new(8, 64, 8);
    let chunk = pending_chest(&world, pos);
    let first = TestPlayer::new(&world).player;
    let second = TestPlayer::new(&world).player;
    world
        .players
        .store(Arc::new(vec![first.clone(), second.clone()]));
    first.get_entity().set_pos(pos.to_centered_f64());
    second.get_entity().set_pos(pos.to_centered_f64());
    let constructed = Arc::new(Barrier::new(2));
    *world.persistence_test_hooks.lazy.lock().unwrap() = Some(Arc::new(move || {
        constructed.wait();
    }));
    let runtime = tokio::runtime::Handle::current();
    let inventories = std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            let _guard = runtime.enter();
            open_chest(&server, &world, &first, pos)
        });
        let two = scope.spawn(|| {
            let _guard = runtime.enter();
            open_chest(&server, &world, &second, pos)
        });
        (one.join().unwrap(), two.join().unwrap())
    });
    *world.persistence_test_hooks.lazy.lock().unwrap() = None;
    assert!(
        Arc::ptr_eq(&inventories.0, &inventories.1),
        "both real chest opens must retain the same Arc"
    );
    assert!(Arc::ptr_eq(
        &inventories.0,
        &world
            .get_block_entity(&pos)
            .unwrap()
            .get_inventory()
            .unwrap()
    ));
    first
        .inventory
        .set_stack(9, ItemStack::new(7, &Item::DIAMOND));
    let sync_id = first
        .current_screen_handler
        .lock()
        .unwrap()
        .lock()
        .unwrap()
        .get_behaviour()
        .sync_id;
    first.on_slot_click(
        SClickSlot {
            sync_id: i32::from(sync_id).into(),
            revision: VarInt(0),
            slot: 27,
            button: 0,
            mode: SlotActionType::QuickMove,
            length_of_array: VarInt(0),
            array_of_changed_slots: Vec::new(),
            carried_item: OptionalItemStackHash(None),
        },
        &server,
    );
    assert_eq!(inventories.1.get_stack(0).item_count, 12);
    assert!(first.inventory.get_stack(9).is_empty());
    first.close_handled_screen();
    second.close_handled_screen();
    unload_and_reload(&world, pos, chunk).await;
    let inventory = open_chest(&server, &world, &first, pos);
    assert_eq!(inventory.get_stack(0).item_count, 12);
    assert_eq!(inventory.get_stack(0).item, &Item::DIAMOND);
    world.level.shutdown().await.unwrap();
}

fn pause_first_lazy_load(world: &World) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (paused, arrived) = mpsc::channel();
    let (resume, proceed) = mpsc::channel();
    let proceed = Mutex::new(proceed);
    let calls = AtomicUsize::new(0);
    *world.persistence_test_hooks.lazy.lock().unwrap() = Some(Arc::new(move || {
        if calls.fetch_add(1, Relaxed) == 0 {
            paused.send(()).unwrap();
            proceed.lock().unwrap().recv().unwrap();
        }
    }));
    (arrived, resume)
}

#[derive(Debug)]
enum LazyInvalidation {
    Remove,
    Unload,
    NbtReplacement,
    ChunkReplacement,
}

#[tokio::test]
async fn invalidated_lazy_chest_snapshot_cannot_publish_after_remove_unload_or_replacement() {
    for invalidation in [
        LazyInvalidation::Remove,
        LazyInvalidation::Unload,
        LazyInvalidation::NbtReplacement,
        LazyInvalidation::ChunkReplacement,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let pos = BlockPos::new(8, 64, 8);
        let chunk = pending_chest(&world, pos);
        let player = TestPlayer::new(&world).player;
        player.get_entity().set_pos(pos.to_centered_f64());
        let original_screen = player.current_screen_handler.lock().unwrap().clone();
        let (arrived, resume) = pause_first_lazy_load(&world);
        let loading = world.clone();
        let loading_player = player.clone();
        let loading_server = server.clone();
        let runtime = tokio::runtime::Handle::current();
        let reader = std::thread::spawn(move || {
            let _guard = runtime.enter();
            use_chest(&loading_server, &loading, &loading_player, pos);
        });
        arrived.recv().unwrap();
        match invalidation {
            LazyInvalidation::Remove => {
                world.set_block_state(
                    &pos,
                    Block::AIR.default_state.id,
                    BlockFlags::NOTIFY_LISTENERS,
                );
            }
            LazyInvalidation::Unload => {
                world
                    .remove_entities_in_chunks([pos.chunk_position()])
                    .await;
                world.tick(&server);
            }
            LazyInvalidation::NbtReplacement => {
                // Even an identical replacement is a different pending record.
                let nbt = chunk.pending_block_entities.lock().unwrap()[&pos].clone();
                world.add_block_entity_nbt(pos, &nbt);
            }
            LazyInvalidation::ChunkReplacement => {
                world.level.loaded_chunks.remove(&pos.chunk_position());
                pending_chest(&world, pos);
            }
        }
        resume.send(()).unwrap();
        reader.join().unwrap();
        assert!(
            world
                .block_entities
                .get(&pos.chunk_position())
                .is_none_or(|entities| !entities.contains_key(&pos)),
            "invalidated {invalidation:?} snapshot was published"
        );
        assert!(Arc::ptr_eq(
            &original_screen,
            &player.current_screen_handler.lock().unwrap()
        ));
        assert!(
            world
                .pending_block_entity_loads
                .lock()
                .unwrap()
                .snapshots
                .is_empty()
        );
        world.level.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn losing_lazy_chest_initializer_preserves_live_custom_data_through_reload() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let pos = BlockPos::new(8, 64, 8);
    let chunk = pending_chest(&world, pos);
    let first = TestPlayer::new(&world).player;
    let second = TestPlayer::new(&world).player;
    world
        .players
        .store(Arc::new(vec![first.clone(), second.clone()]));
    first.get_entity().set_pos(pos.to_centered_f64());
    second.get_entity().set_pos(pos.to_centered_f64());
    let mut namespace = NbtCompound::new();
    namespace.put_int("revision", 1);
    namespace.put_double("nan", f64::NAN);
    let mut custom = NbtCompound::new();
    custom.put_compound("test", namespace);
    chunk
        .pending_block_entities
        .lock()
        .unwrap()
        .get_mut(&pos)
        .unwrap()
        .put_compound("PumpkinCustomData", custom);
    let (arrived, resume) = pause_first_lazy_load(&world);
    let loading = world.clone();
    let loading_server = server.clone();
    let loading_player = first.clone();
    let runtime = tokio::runtime::Handle::current();
    let reader = std::thread::spawn(move || {
        let _guard = runtime.enter();
        open_chest(&loading_server, &loading, &loading_player, pos)
    });
    arrived.recv().unwrap();
    let winner = open_chest(&server, &world, &second, pos);
    world.set_block_entity_custom_data(&pos, "test", "revision", pumpkin_nbt::tag::NbtTag::Int(2));
    resume.send(()).unwrap();
    assert!(Arc::ptr_eq(&winner, &reader.join().unwrap()));
    assert_eq!(
        world.get_block_entity_custom_data(&pos, "test", "revision"),
        Some(pumpkin_nbt::tag::NbtTag::Int(2)),
        "losing initializer overwrote live plugin data"
    );
    *world.persistence_test_hooks.lazy.lock().unwrap() = None;
    first.close_handled_screen();
    second.close_handled_screen();
    unload_and_reload(&world, pos, chunk).await;
    // Remove the runtime cache so only the data actually written to the chunk can restore it.
    world.custom_block_entity_data.remove(&pos);
    open_chest(&server, &world, &first, pos);
    assert_eq!(
        world.get_block_entity_custom_data(&pos, "test", "revision"),
        Some(pumpkin_nbt::tag::NbtTag::Int(2))
    );
    assert!(
        matches!(world.get_block_entity_custom_data(&pos, "test", "nan"), Some(pumpkin_nbt::tag::NbtTag::Double(value)) if value.is_nan())
    );
    world.level.shutdown().await.unwrap();
}

struct ForceDuringTick {
    entity: Entity,
    completed: AtomicBool,
}

impl EntityBase for ForceDuringTick {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }
    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn tick(&self, _caller: &dyn EntityBase, _server: &Server) {
        let world = self.entity.world.load_full();
        // Bound by attempts, so the old read guard produces a failure instead of a hung suite.
        for _ in 0..3 {
            if let Ok(guard) = world.active_chunks.try_write() {
                drop(guard);
                world.set_chunks_forced(&[Vector2::new(1, 0)], true);
                self.completed.store(true, Relaxed);
                break;
            }
        }
    }
}

#[tokio::test]
async fn world_tick_callback_can_force_a_chunk_without_retained_read_guard() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    publish_empty_chunk(&world, Vector2::new(0, 0));
    world
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));
    let entity = Arc::new(ForceDuringTick {
        entity: Entity::new(
            world.clone(),
            Vector3::new(8.0, 64.0, 8.0),
            &EntityType::MARKER,
        ),
        completed: AtomicBool::new(false),
    });
    world.add_entity_silent(entity.clone());
    world.tick(&server);
    assert!(
        entity.completed.load(Relaxed),
        "entity callback could not acquire active membership for forceload"
    );
    assert!(
        world
            .forced_chunks
            .lock()
            .unwrap()
            .contains(&Vector2::new(1, 0))
    );
    assert!(
        world
            .active_chunks
            .read()
            .unwrap()
            .contains(&Vector2::new(1, 0))
    );
    world.level.shutdown().await.unwrap();
}
