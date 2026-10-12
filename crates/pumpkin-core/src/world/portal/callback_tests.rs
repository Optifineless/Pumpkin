#![expect(
    clippy::unwrap_used,
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
use pumpkin_world::world::BlockFlags;

use super::{
    PortalType,
    poi::POI_TYPE_NETHER_PORTAL,
    tests::{save_destination, save_destination_with_chest},
};
use crate::{
    block::entities::{BlockEntity, chest::ChestBlockEntity},
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
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_callbacks_preserve_new_container_and_drop_only_retired_contents() {
    for live in [true, false] {
        let fixture = DeathTestWorld::new().await;
        let source = fixture.world();
        let destination = fixture
            .server
            .get_world_from_dimension(&Dimension::THE_NETHER);
        // The last fallback ground write is delivered after the first physics callback.
        let pos = BlockPos::new(1, 79, 2);
        let old = Arc::new(ChestBlockEntity::new(pos));
        old.set_stack(0, ItemStack::new(1, &Item::EMERALD));
        let mut nbt = NbtCompound::new();
        old.write_internal(&mut nbt);
        save_destination_with_chest(&destination, false, Some((pos, nbt))).await;
        assert!(destination.level.loaded_chunks.is_empty());
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
            "deferred cleanup removed the callback's new chest (live={live})"
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
        assert_eq!(drops, [(Item::EMERALD.id, 1)], "live={live}");
        fixture.server.shutdown().await;
    }
}
