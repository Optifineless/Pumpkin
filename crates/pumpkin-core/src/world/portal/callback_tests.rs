#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Regression fixtures require successful construction"
)]

use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::future::BoxFuture;
use pumpkin_data::{
    Block, BlockDirection, dimension::Dimension, entity::EntityType, item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::{
    chunk::io::{FileIO, LoadedData},
    world::BlockFlags,
};

use super::{
    PortalType,
    poi::POI_TYPE_NETHER_PORTAL,
    residency::PortalChunkResidency,
    tests::{save_destination, save_destination_with_block},
};
use crate::{
    block::entities::{BlockEntity, chest::ChestBlockEntity, create_block_entity},
    entity::{EntityBase, death_test_world::DeathTestWorld, item::ItemEntity},
    plugin::{EventHandler, EventPriority, api::events::block::block_physics::BlockPhysicsEvent},
    server::Server,
    world::World,
};

struct ReentrantPhysics {
    source: Arc<World>,
    destination: Arc<World>,
    traveler: Arc<dyn EntityBase>,
    callbacks: AtomicUsize,
    arrival: OnceLock<Vector3<f64>>,
    replacement: Option<Arc<dyn BlockEntity>>,
}

impl EventHandler<BlockPhysicsEvent> for ReentrantPhysics {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut BlockPhysicsEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if self.callbacks.fetch_add(1, Ordering::Relaxed) != 0 {
                return;
            }
            // The real callback forces lookup while the outer creator waits for it.
            // This deadline only breaks a deadlock; success is the nested destination.
            let result = tokio::time::timeout(
                Duration::from_secs(30),
                PortalType::Nether.get_portal_destination(
                    &self.source,
                    self.destination.clone(),
                    self.traveler.as_ref(),
                    None,
                ),
            )
            .await;
            assert!(
                result.is_ok(),
                "reentrant portal lookup hit the 30 s crash guard"
            );
            let transition = result.unwrap().unwrap();
            assert!(self.arrival.set(transition.position).is_ok());
            assert_frame(&self.destination);
            if let Some(replacement) = &self.replacement {
                let pos = replacement.get_position();
                self.destination.set_block_state(
                    &pos,
                    Block::CHEST.default_state.id,
                    BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
                );
                self.destination.add_block_entity(replacement.clone());
            }
        })
    }
}

// PortalForcer.createPortal's empty-terrain fallback at the scaled (1,80,1) exit.
fn assert_frame(world: &World) {
    let positions = world.portal_poi.lock().unwrap().get_in_square(
        BlockPos::new(1, 80, 1),
        super::nether::SEARCH_RADIUS_NETHER,
        Some(POI_TYPE_NETHER_PORTAL),
    );
    assert_eq!(
        positions.len(),
        6,
        "exactly one 2 by 3 portal is registered"
    );
    assert!(
        positions
            .iter()
            .all(|pos| world.get_block(pos) == &Block::NETHER_PORTAL)
    );
    let mut obsidian = 0;
    let mut portal = 0;
    for x in -1..=2 {
        for y in 79..=83 {
            for z in 0..=2 {
                match world.get_block(&BlockPos::new(x, y, z)) {
                    block if block == &Block::OBSIDIAN => obsidian += 1,
                    block if block == &Block::NETHER_PORTAL => portal += 1,
                    block => assert_eq!(block, &Block::AIR),
                }
            }
        }
    }
    assert_eq!((obsidian, portal), (18, 6));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_creation_releases_ownership_before_reentrant_physics() {
    let fixture = DeathTestWorld::new().await;
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    save_destination(&destination, false).await;
    assert!(destination.level.loaded_chunks.is_empty());
    let residency = retain_frame(&destination);
    let traveler = fixture.mob(&EntityType::COW);
    traveler.get_entity().set_pos(Vector3::new(8.0, 80.0, 8.0));
    let handler = Arc::new(ReentrantPhysics {
        source: source.clone(),
        destination: destination.clone(),
        traveler: traveler.clone(),
        callbacks: AtomicUsize::new(0),
        arrival: OnceLock::new(),
        replacement: None,
    });
    fixture
        .server
        .plugin_manager
        .register::<BlockPhysicsEvent, _>(handler.clone(), EventPriority::Normal, true);
    let transition = tokio::time::timeout(
        Duration::from_secs(60),
        PortalType::Nether.get_portal_destination(
            &source,
            destination.clone(),
            traveler.as_ref(),
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(handler.arrival.get(), Some(&transition.position));
    assert_frame(&destination);
    // One fallback creates 18 distinct obsidian blocks, each notifying six neighbours.
    // A second build would clear the first interior and emit additional physics events.
    assert_eq!(
        handler.callbacks.load(Ordering::Relaxed),
        18 * BlockDirection::update_order().len()
    );
    assert!(destination.portal_creation.try_lock().is_ok());
    drop(residency);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_callbacks_preserve_new_container_and_drop_only_retired_contents() {
    for live in [true, false] {
        assert_frame_preserves_container(&Block::CHEST, live).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_callbacks_preserve_live_sign_replacement() {
    for block in [&Block::OAK_SIGN, &Block::OAK_HANGING_SIGN] {
        assert_frame_preserves_container(block, true).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_callbacks_preserve_pending_sign_replacement() {
    for block in [&Block::OAK_SIGN, &Block::OAK_HANGING_SIGN] {
        assert_frame_preserves_container(block, false).await;
    }
}

async fn assert_frame_preserves_container(old_block: &Block, live: bool) {
    let fixture = DeathTestWorld::new().await;
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    // The last fallback ground write is delivered after the first physics callback.
    let pos = BlockPos::new(1, 79, 2);
    save_retired_block(&destination, pos, old_block).await;
    assert!(destination.level.loaded_chunks.is_empty());
    let residency = retain_frame(&destination);
    if live {
        destination
            .level
            .get_or_fetch_chunk(pos.chunk_position(), |_| ())
            .await
            .unwrap();
        assert!(destination.get_block_entity(&pos).is_some());
    }
    let replacement = Arc::new(ChestBlockEntity::new(pos));
    replacement.set_stack(0, ItemStack::new(1, &Item::DIAMOND));
    let replacement: Arc<dyn BlockEntity> = replacement;
    let traveler = fixture.mob(&EntityType::COW);
    traveler.get_entity().set_pos(Vector3::new(8.0, 80.0, 8.0));
    let handler = Arc::new(ReentrantPhysics {
        source: source.clone(),
        destination: destination.clone(),
        traveler: traveler.clone(),
        callbacks: AtomicUsize::new(0),
        arrival: OnceLock::new(),
        replacement: Some(replacement.clone()),
    });
    fixture
        .server
        .plugin_manager
        .register::<BlockPhysicsEvent, _>(handler.clone(), EventPriority::Normal, true);
    let transition = tokio::time::timeout(
        Duration::from_secs(60),
        PortalType::Nether.get_portal_destination(
            &source,
            destination.clone(),
            traveler.as_ref(),
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(handler.arrival.get(), Some(&transition.position));
    let current = destination.get_block_entity(&pos);
    assert!(
        current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &replacement)),
        "deferred cleanup removed the callback's new chest (old={}, live={live})",
        old_block.name
    );
    let kept = replacement.clone().get_inventory().unwrap().get_stack(0);
    assert_eq!((kept.item.id, kept.item_count), (Item::DIAMOND.id, 1));
    let drops: Vec<_> = destination
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.cast_any().downcast_ref::<ItemEntity>())
        .map(|entity| {
            let stack = entity.get_item_stack().lock().unwrap();
            (stack.item.id, stack.item_count)
        })
        .collect();
    let expected = if old_block == &Block::CHEST {
        vec![(Item::EMERALD.id, 1)]
    } else {
        Vec::new()
    };
    assert_eq!(drops, expected, "old={}, live={live}", old_block.name);
    tokio::time::timeout(
        Duration::from_secs(60),
        assert_saved_chest(&destination, pos),
    )
    .await
    .unwrap();
    drop(residency);
    fixture.server.shutdown().await;
}

async fn assert_saved_chest(world: &Arc<World>, pos: BlockPos) {
    // Exercise real unload serialization, disk saving, chunk fetch and lazy promotion.
    let chunk = world
        .level
        .loaded_chunks
        .get(&pos.chunk_position())
        .unwrap()
        .clone();
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
    world
        .level
        .chunk_saver
        .block_and_await_ongoing_tasks()
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
        panic!("saved replacement chest chunk missing")
    };
    world
        .level
        .loaded_chunks
        .insert(pos.chunk_position(), reloaded);
    assert_eq!(world.get_block(&pos), &Block::CHEST);
    let inventory = world
        .get_block_entity(&pos)
        .unwrap()
        .get_inventory()
        .unwrap();
    let stack = inventory.get_stack(0);
    assert_eq!((stack.item.id, stack.item_count), (Item::DIAMOND.id, 1));
}

async fn save_retired_block(world: &World, pos: BlockPos, block: &Block) {
    let mut nbt = NbtCompound::new();
    if block == &Block::CHEST {
        let old = ChestBlockEntity::new(pos);
        old.set_stack(0, ItemStack::new(1, &Item::EMERALD));
        old.write_internal(&mut nbt);
    } else {
        // Vanilla and older fork saves use these sign IDs and coordinate fields.
        create_block_entity(block.default_state.block_entity_type, pos)
            .unwrap()
            .write_internal(&mut nbt);
    }
    save_destination_with_block(world, false, Some((pos, block, nbt))).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_callbacks_preserve_rebuilt_portal_poi() {
    let fixture = DeathTestWorld::new().await;
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    // Invalid fragment in the fallback interior: construction clears then rebuilds it.
    // No old POI means the real lookup misses and selects fallback construction.
    save_destination_with_block(
        &destination,
        false,
        Some((
            BlockPos::new(1, 80, 1),
            &Block::NETHER_PORTAL,
            NbtCompound::new(),
        )),
    )
    .await;
    let residency = retain_frame(&destination);
    let traveler = fixture.mob(&EntityType::COW);
    traveler.get_entity().set_pos(Vector3::new(8.0, 80.0, 8.0));
    PortalType::Nether
        .get_portal_destination(&source, destination.clone(), traveler.as_ref(), None)
        .await
        .unwrap();
    assert_frame(&destination);
    drop(residency);
    fixture.server.shutdown().await;
}

fn retain_frame(world: &World) -> PortalChunkResidency {
    // Retain fixture terrain through assertions and the disk round trip, without
    // relying on the scheduler's timing after destination creation releases tickets.
    let mut residency = PortalChunkResidency::new(world.level.clone());
    residency.add_area(BlockPos::new(1, 80, 1), 3);
    residency
}
