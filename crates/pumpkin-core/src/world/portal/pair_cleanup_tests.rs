#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures require successful construction"
)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::future::BoxFuture;
use pumpkin_data::{
    Block, BlockStateId,
    block_properties::{
        BedPart, DoubleBlockHalf, HorizontalFacing, OakDoorLikeProperties, WhiteBedLikeProperties,
    },
    dimension::Dimension,
    entity::EntityType,
    tag::Taggable,
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

use super::{
    PortalType, poi::POI_TYPE_NETHER_PORTAL, residency::PortalChunkResidency,
    tests::save_destination,
};
use crate::{
    entity::death_test_world::DeathTestWorld,
    plugin::{EventHandler, EventPriority, api::events::block::block_physics::BlockPhysicsEvent},
    server::Server,
    world::World,
};

const RETIRED: BlockPos = BlockPos::new(1, 82, 2);

struct Pair {
    halves: [(BlockPos, BlockStateId); 2],
}

impl Pair {
    fn new(block: &Block, replacement: bool) -> Self {
        let halves = if block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_DOORS) {
            let mut props = OakDoorLikeProperties::default(block);
            props.facing = if replacement {
                HorizontalFacing::East
            } else {
                HorizontalFacing::South
            };
            props.half = DoubleBlockHalf::Lower;
            let lower = (RETIRED, props.to_state_id(block));
            props.half = DoubleBlockHalf::Upper;
            [lower, (RETIRED.up(), props.to_state_id(block))]
        } else {
            let mut props = WhiteBedLikeProperties::default(block);
            props.facing = if replacement {
                HorizontalFacing::East
            } else {
                HorizontalFacing::South
            };
            props.part = BedPart::Foot;
            let foot = if replacement {
                BlockPos::new(0, 82, 3)
            } else {
                RETIRED
            };
            let lower = (foot, props.to_state_id(block));
            props.part = BedPart::Head;
            [lower, (BlockPos::new(1, 82, 3), props.to_state_id(block))]
        };
        Self { halves }
    }

    fn install(&self, world: &Arc<World>) {
        // Publish a complete supported pair without notifying its incomplete first half.
        let flags = BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE;
        for (pos, _) in self.halves {
            world.set_block_state(&pos.down(), Block::STONE.default_state.id, flags);
        }
        for (pos, state) in self.halves {
            world.set_block_state(&pos, state, flags);
        }
    }
}

struct ReplacePair {
    world: Arc<World>,
    replacement: Pair,
    installed: AtomicBool,
    retired_notifications: AtomicUsize,
}

impl EventHandler<BlockPhysicsEvent> for ReplacePair {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut BlockPhysicsEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.changed_pos == RETIRED {
                self.retired_notifications.fetch_add(1, Ordering::Relaxed);
            }
            if !self.installed.swap(true, Ordering::Relaxed) {
                assert_eq!(event.changed_pos, BlockPos::new(0, 79, 0));
                assert_eq!(self.world.get_block(&RETIRED), &Block::AIR);
                assert_complete_frame(&self.world);
                // First obsidian physics event precedes the retired side-cell callback.
                self.replacement.install(&self.world);
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_callbacks_preserve_replacement_door_pair() {
    exercise_pair_cleanup(&Block::OAK_DOOR, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_callbacks_preserve_replacement_copper_door_pair() {
    exercise_pair_cleanup(&Block::COPPER_DOOR, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_callbacks_preserve_replacement_bed_pair() {
    exercise_pair_cleanup(&Block::RED_BED, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_callbacks_preserve_replacement_straw_bed_pair() {
    exercise_pair_cleanup(&Block::STRAW_BED, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_callbacks_remove_unchanged_door_partner() {
    exercise_pair_cleanup(&Block::OAK_DOOR, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_callbacks_remove_unchanged_bed_partner() {
    exercise_pair_cleanup(&Block::RED_BED, false).await;
}

async fn exercise_pair_cleanup(block: &Block, replace: bool) {
    let fixture = DeathTestWorld::new().await;
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    save_destination(&destination, false).await;
    let mut residency = PortalChunkResidency::new(destination.level.clone());
    residency.add_area(BlockPos::new(1, 80, 1), 3);
    destination
        .level
        .get_or_fetch_chunk(RETIRED.chunk_position(), |_| ())
        .await
        .unwrap();
    let retired = Pair::new(block, false);
    retired.install(&destination);
    let handler = Arc::new(ReplacePair {
        world: destination.clone(),
        replacement: Pair::new(block, true),
        installed: AtomicBool::new(false),
        retired_notifications: AtomicUsize::new(0),
    });
    if replace {
        fixture
            .server
            .plugin_manager
            .register::<BlockPhysicsEvent, _>(handler.clone(), EventPriority::Normal, true);
    }
    let traveler = fixture.mob(&EntityType::COW);
    traveler.get_entity().set_pos(Vector3::new(8.0, 80.0, 8.0));
    // Deadline is a crash guard; the first real physics event forces the interleaving.
    tokio::time::timeout(
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
    assert_complete_frame(&destination);
    if replace {
        assert!(handler.installed.load(Ordering::Relaxed));
        for (pos, state) in handler.replacement.halves {
            assert_eq!(
                destination.get_block_state_id(&pos),
                state,
                "deferred cleanup broke the plugin's {} at {pos:?}",
                block.name
            );
        }
        assert!(
            handler.retired_notifications.load(Ordering::Relaxed) > 0,
            "retired pair still delivers non-destructive physics notifications"
        );
    } else {
        for (pos, _) in retired.halves {
            assert_eq!(
                destination.get_block(&pos),
                &Block::AIR,
                "unchanged {} partner remains at {pos:?}",
                block.name
            );
        }
    }
    drop(residency);
    fixture.server.shutdown().await;
}

fn assert_complete_frame(world: &World) {
    for x in -1..=2 {
        for y in 79..=83 {
            let expected = if x == -1 || x == 2 || y == 79 || y == 83 {
                &Block::OBSIDIAN
            } else {
                &Block::NETHER_PORTAL
            };
            assert_eq!(world.get_block(&BlockPos::new(x, y, 1)), expected);
        }
    }
    let pois = world.portal_poi.lock().unwrap().get_in_square(
        BlockPos::new(1, 80, 1),
        3,
        Some(POI_TYPE_NETHER_PORTAL),
    );
    assert_eq!(pois.len(), 6);
}
