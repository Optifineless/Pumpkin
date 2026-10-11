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
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, Ordering::Relaxed},
};

fn open_chest(
    server: &Arc<Server>,
    world: &Arc<World>,
    player: &Arc<crate::entity::player::Player>,
    pos: BlockPos,
) -> Arc<dyn Inventory> {
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
    world: &World,
    pos: BlockPos,
    chunk: Arc<pumpkin_world::chunk::ChunkData>,
) {
    world
        .remove_entities_in_chunks([pos.chunk_position()])
        .await;
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
    *world.lazy_block_entity_hook.lock().unwrap() = Some(Arc::new(move |_| {
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
    *world.lazy_block_entity_hook.lock().unwrap() = None;
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
