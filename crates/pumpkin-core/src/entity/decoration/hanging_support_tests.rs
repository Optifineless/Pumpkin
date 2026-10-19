#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Real-entry regression fixtures require successful bounded setup"
)]

use super::{hanging_test_support::Fixture, item_frame::ItemFrameEntity};
use crate::entity::EntityBase;
use pumpkin_data::{
    Block, BlockDirection, item::Item, item_stack::ItemStack, painting_variant::PaintingVariant,
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use std::time::Duration;

struct CancelSupport(
    std::sync::atomic::AtomicBool,
    std::sync::atomic::AtomicUsize,
);

impl crate::plugin::EventHandler<crate::plugin::api::events::hanging::HangingBreakEvent>
    for CancelSupport
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a std::sync::Arc<crate::server::Server>,
        event: &'a mut crate::plugin::api::events::hanging::HangingBreakEvent,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        assert!(
            event.remover.is_none(),
            "support retirement has no attacker"
        );
        assert!(
            !event.entity.get_entity().is_removed(),
            "event precedes removal"
        );
        self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        event.cancelled = self.0.load(std::sync::atomic::Ordering::Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_support_event_cancellation_keeps_entity_and_retries() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let fixture = Fixture::new();
    let support = BlockPos::new(8, 64, 8);
    fixture.set_block(support, &Block::STONE);
    let painting = fixture.place(
        &Item::PAINTING,
        support,
        BlockDirection::South,
        Some(PaintingVariant::Alban),
    );
    let cancel = Arc::new(CancelSupport(AtomicBool::new(true), AtomicUsize::new(0)));
    fixture
        .server
        .plugin_manager
        .register::<crate::plugin::api::events::hanging::HangingBreakEvent, _>(
            cancel.clone(),
            crate::plugin::EventPriority::Normal,
            true,
        );
    fixture.tick(101);
    assert_eq!(
        cancel.1.load(Ordering::Relaxed),
        0,
        "supported control emits no break event"
    );
    fixture.set_block(support, &Block::AIR);
    fixture.tick(101);
    assert!(
        !painting.get_entity().is_removed(),
        "cancelled support event preserves painting"
    );
    assert_eq!(fixture.drops(&Item::PAINTING), 0);
    assert_eq!(cancel.1.load(Ordering::Relaxed), 1);
    cancel.0.store(false, Ordering::Relaxed);
    fixture.tick(101);
    assert!(
        painting.get_entity().is_removed(),
        "next bounded check retries cancelled retirement"
    );
    assert_eq!(fixture.drops(&Item::PAINTING), 1);
    assert_eq!(cancel.1.load(Ordering::Relaxed), 2);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_support_loss_uses_world_tick_and_one_drop() {
    let fixture = Fixture::new();
    let frame_wall = BlockPos::new(6, 64, 8);
    let painting_wall = BlockPos::new(10, 64, 8);
    for pos in [frame_wall, painting_wall] {
        fixture.set_block(pos, &Block::STONE);
    }
    let frame = fixture.place(&Item::ITEM_FRAME, frame_wall, BlockDirection::South, None);
    let painting = fixture.place(
        &Item::PAINTING,
        painting_wall,
        BlockDirection::South,
        Some(PaintingVariant::Alban),
    );
    let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
    concrete.set_item(ItemStack::new(1, &Item::DIAMOND), true);
    fixture.tick(101);
    assert!(!frame.get_entity().is_removed(), "supported frame control");
    assert!(
        !painting.get_entity().is_removed(),
        "supported painting control"
    );
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
    assert_eq!(fixture.drops(&Item::PAINTING), 0);
    for pos in [frame_wall, painting_wall] {
        fixture.set_block(pos, &Block::AIR);
    }
    fixture.tick(100);
    assert!(
        !frame.get_entity().is_removed(),
        "check must not run before tick 101"
    );
    assert!(
        !painting.get_entity().is_removed(),
        "check must not run before tick 101"
    );
    fixture.tick(1);
    assert!(
        frame.get_entity().is_removed(),
        "unsupported ordinary frame must retire at next check"
    );
    assert!(
        painting.get_entity().is_removed(),
        "unsupported painting must retire at next check"
    );
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    assert_eq!(fixture.drops(&Item::PAINTING), 1);
    assert_eq!(fixture.drops(&Item::DIAMOND), 1);
    assert!(concrete.get_item().is_empty());
    fixture.tick(101);
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    assert_eq!(fixture.drops(&Item::PAINTING), 1);
    assert_eq!(fixture.drops(&Item::DIAMOND), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_fixed_frame_is_immune_to_support_loss() {
    let fixture = Fixture::new();
    let support = BlockPos::new(8, 64, 8);
    fixture.set_block(support, &Block::STONE);
    let frame = fixture.place(&Item::ITEM_FRAME, support, BlockDirection::South, None);
    let frame = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
    frame.set_fixed(true);
    frame.set_item(ItemStack::new(1, &Item::DIAMOND), true);
    fixture.tick(101);
    fixture.set_block(support, &Block::AIR);
    fixture.tick(202);
    assert!(
        !frame.get_entity().is_removed(),
        "Fixed frame survives without support"
    );
    assert_eq!(frame.get_item().item, &Item::DIAMOND);
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
    assert_eq!(fixture.drops(&Item::DIAMOND), 0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_multiblock_painting_loses_partial_support() {
    let fixture = Fixture::new();
    let variant = PaintingVariant::all()
        .iter()
        .copied()
        .find(|variant| variant.width() == 4 && variant.height() == 4)
        .unwrap();
    for x in 7..=10 {
        for y in 64..=67 {
            fixture.set_block(BlockPos::new(x, y, 8), &Block::STONE);
        }
    }
    let painting = fixture.place(
        &Item::PAINTING,
        BlockPos::new(8, 65, 8),
        BlockDirection::South,
        Some(variant),
    );
    fixture.tick(101);
    assert!(
        !painting.get_entity().is_removed(),
        "all sixteen support cells control"
    );
    fixture.set_block(BlockPos::new(7, 67, 8), &Block::AIR);
    fixture.tick(101);
    assert!(
        painting.get_entity().is_removed(),
        "loss of outer corner support must retire full painting"
    );
    assert_eq!(fixture.drops(&Item::PAINTING), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_hanging_obstruction_and_coexistence() {
    let fixture = Fixture::new();
    for x in [6, 10] {
        fixture.set_block(BlockPos::new(x, 64, 8), &Block::STONE);
    }
    let frame = fixture.place(
        &Item::ITEM_FRAME,
        BlockPos::new(6, 64, 8),
        BlockDirection::South,
        None,
    );
    let painting = fixture.place(
        &Item::PAINTING,
        BlockPos::new(10, 64, 8),
        BlockDirection::South,
        Some(PaintingVariant::Alban),
    );
    fixture.tick(101);
    assert!(!frame.get_entity().is_removed());
    assert!(!painting.get_entity().is_removed());
    fixture.set_block(BlockPos::new(6, 64, 9), &Block::OAK_TRAPDOOR);
    fixture.set_block(BlockPos::new(10, 64, 9), &Block::STONE);
    fixture.tick(101);
    assert!(
        frame.get_entity().is_removed(),
        "non-solid collision shape obstructs frame"
    );
    assert!(
        painting.get_entity().is_removed(),
        "collision shape obstructs painting"
    );
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    assert_eq!(fixture.drops(&Item::PAINTING), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_overlapping_hanging_entities_retire_without_duplicate_drops() {
    use super::painting::PaintingEntity;
    let fixture = Fixture::new();
    for x in 4..=11 {
        for y in 62..=68 {
            fixture.set_block(BlockPos::new(x, y, 8), &Block::STONE);
        }
    }
    let painting = fixture.place(
        &Item::PAINTING,
        BlockPos::new(8, 65, 8),
        BlockDirection::South,
        Some(PaintingVariant::Alban),
    );
    let frame = fixture.place(
        &Item::ITEM_FRAME,
        BlockPos::new(7, 65, 8),
        BlockDirection::South,
        None,
    );
    fixture.tick(101);
    assert!(
        !painting.get_entity().is_removed(),
        "separated painting control"
    );
    assert!(!frame.get_entity().is_removed(), "separated frame control");
    let variant = PaintingVariant::all()
        .iter()
        .copied()
        .find(|variant| variant.width() == 4 && variant.height() == 4)
        .unwrap();
    painting
        .cast_any()
        .downcast_ref::<PaintingEntity>()
        .unwrap()
        .set_variant(variant);
    fixture.tick(101);
    assert!(
        painting.get_entity().is_removed() || frame.get_entity().is_removed(),
        "intersecting same-facing hanging entities cannot coexist"
    );
    let painting_drops = fixture.drops(&Item::PAINTING);
    let frame_drops = fixture.drops(&Item::ITEM_FRAME);
    assert_eq!(
        painting_drops,
        u32::from(painting.get_entity().is_removed())
    );
    assert_eq!(frame_drops, u32::from(frame.get_entity().is_removed()));
    fixture.tick(101);
    assert_eq!(fixture.drops(&Item::PAINTING), painting_drops);
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), frame_drops);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_collision_from_lower_neighbor_retires_frame() {
    use pumpkin_data::block_properties::OakFenceLikeProperties;
    use pumpkin_world::world::BlockFlags;
    let fixture = Fixture::new();
    let support = BlockPos::new(8, 64, 8);
    fixture.set_block(support, &Block::STONE);
    let frame = fixture.place(&Item::ITEM_FRAME, support, BlockDirection::South, None);
    fixture.tick(101);
    assert!(!frame.get_entity().is_removed(), "unobstructed control");
    let mut fence = OakFenceLikeProperties::default(&Block::OAK_FENCE);
    fence.north = true;
    fixture.world.set_block_state(
        &BlockPos::new(8, 63, 9),
        fence.to_state_id(&Block::OAK_FENCE),
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    fixture.tick(101);
    assert!(
        frame.get_entity().is_removed(),
        "lower fence collision extends into the hanging pop box"
    );
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_unload_reload_keeps_live_decoration_and_never_revives_broken_one() {
    let fixture = Fixture::new();
    let support = BlockPos::new(8, 64, 8);
    fixture.set_block(support, &Block::STONE);
    let painting = fixture.place(
        &Item::PAINTING,
        support,
        BlockDirection::South,
        Some(PaintingVariant::Alban),
    );
    let id = painting.get_entity().entity_uuid;
    fixture.tick(101);
    let pos = Vector2::new(0, 0);
    let storage = tokio::time::timeout(
        Duration::from_secs(30),
        fixture.world.level.get_entity_chunk(pos),
    )
    .await
    .unwrap()
    .unwrap();
    fixture.world.make_chunk_entities_live(&storage, None);
    tokio::time::timeout(
        Duration::from_secs(30),
        fixture.world.remove_entities_in_chunks([pos]),
    )
    .await
    .unwrap();
    fixture.tick(1);
    assert!(
        painting.get_entity().is_removed(),
        "old chunk incarnation retired"
    );
    assert_eq!(
        fixture.drops(&Item::PAINTING),
        0,
        "unload must not break decoration"
    );
    let records = storage.data.lock().unwrap().clone();
    assert_eq!(records.len(), 1);
    // Ordinary entity chunk loading constructs a fresh runtime check counter from saved NBT.
    tokio::time::timeout(
        Duration::from_secs(30),
        fixture.world.level.mark_chunks_as_newly_watched(&[pos]),
    )
    .await
    .unwrap();
    let next = tokio::time::timeout(
        Duration::from_secs(30),
        fixture.world.level.get_entity_chunk(pos),
    )
    .await
    .unwrap()
    .unwrap();
    fixture.world.make_chunk_entities_live(&next, None);
    let loaded = fixture
        .world
        .get_entity_by_uuid(id)
        .expect("saved painting must reload");
    assert!(!loaded.get_entity().is_removed());
    fixture.set_block(support, &Block::AIR);
    fixture.tick(101);
    assert!(
        loaded.get_entity().is_removed(),
        "reloaded painting still validates support"
    );
    assert_eq!(fixture.drops(&Item::PAINTING), 1);
    tokio::time::timeout(
        Duration::from_secs(30),
        fixture.world.level.mark_chunks_as_not_watched([pos]),
    )
    .await
    .unwrap();
    tokio::time::timeout(
        Duration::from_secs(30),
        fixture.world.remove_entities_in_chunks([pos]),
    )
    .await
    .unwrap();
    fixture.tick(1);
    tokio::time::timeout(
        Duration::from_secs(30),
        fixture.world.level.mark_chunks_as_newly_watched(&[pos]),
    )
    .await
    .unwrap();
    let final_chunk = tokio::time::timeout(
        Duration::from_secs(30),
        fixture.world.level.get_entity_chunk(pos),
    )
    .await
    .unwrap()
    .unwrap();
    fixture.world.make_chunk_entities_live(&final_chunk, None);
    assert!(
        fixture.world.get_entity_by_uuid(id).is_none(),
        "broken painting must not revive after reload"
    );
    fixture.finish().await;
}
