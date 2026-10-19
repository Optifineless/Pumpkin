#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Real-entry regressions require successful bounded fixture setup"
)]

use super::{hanging_test_support::Fixture, item_frame::ItemFrameEntity};
use crate::entity::{Entity, EntityBase, living::LivingEntity};
use pumpkin_data::{
    Block, BlockDirection, entity::EntityType, item::Item, item_stack::ItemStack,
    painting_variant::PaintingVariant,
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct TypeProbe {
    entity: Entity,
    casts: AtomicUsize,
}

impl EntityBase for TypeProbe {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }
    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self.casts.fetch_add(1, Ordering::Relaxed);
        self
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_many_frames_prefilter_distant_entities() {
    for count in [1, 32] {
        let fixture = Fixture::new();
        for index in 0..count {
            let support = BlockPos::new(2 + index % 8, 65 + index / 8, 8);
            fixture.player.player.get_entity().set_pos(Vector3::new(
                f64::from(support.0.x) + 0.5,
                f64::from(support.0.y) - 1.0,
                12.5,
            ));
            fixture.set_block(support, &Block::STONE);
            fixture.place(&Item::ITEM_FRAME, support, BlockDirection::South, None);
        }
        let probe = Arc::new(TypeProbe {
            entity: Entity::new(
                fixture.world.clone(),
                Vector3::new(1000.0, 65.0, 1000.0),
                &EntityType::ARMOR_STAND,
            ),
            casts: AtomicUsize::new(0),
        });
        fixture.world.entities.rcu(|entities| {
            let mut entities = (**entities).clone();
            entities.push(probe.clone());
            entities
        });
        fixture.tick(100);
        probe.casts.store(0, Ordering::Relaxed);
        fixture.tick(1);
        // The world orb snapshot performs one baseline type query per entity.
        assert_eq!(
            probe.casts.load(Ordering::Relaxed),
            1,
            "distant entity geometry work scaled with {count} frames"
        );
        assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_variant_parity_preserves_attachment() {
    for face in [
        BlockDirection::North,
        BlockDirection::South,
        BlockDirection::East,
        BlockDirection::West,
    ] {
        let fixture = Fixture::new();
        let support = BlockPos::new(8, 64, 8);
        let left = face.rotate_counter_clockwise().to_offset();
        for x in 0..2 {
            for y in 0..2 {
                fixture.set_block(
                    support.offset(left * x).offset(Vector3::new(0, y, 0)),
                    &Block::STONE,
                );
            }
        }
        let painting = fixture.place(&Item::PAINTING, support, face, Some(PaintingVariant::Alban));
        let original = painting.get_entity().pos.load();
        for (name, shift) in [
            ("minecraft:pool", left.to_f64() * 0.5),
            ("minecraft:alban", Vector3::new(0.0, 0.0, 0.0)),
            ("minecraft:wanderer", Vector3::new(0.0, 0.5, 0.0)),
            (
                "minecraft:bust",
                left.to_f64() * 0.5 + Vector3::new(0.0, 0.5, 0.0),
            ),
        ] {
            painting.set_variant_name(name);
            assert_eq!(
                painting.get_entity().pos.load(),
                original + shift,
                "{face:?} {name} center"
            );
            fixture.tick(101);
            assert!(
                !painting.get_entity().is_removed(),
                "variant checked the wrong support cells"
            );
        }
        painting.set_variant_name("minecraft:alban");
        for (x, y) in [(1, 0), (0, 1), (1, 1)] {
            fixture.set_block(
                support.offset(left * x).offset(Vector3::new(0, y, 0)),
                &Block::AIR,
            );
        }
        assert_eq!(painting.get_entity().pos.load(), original);
        fixture.tick(101);
        assert!(!painting.get_entity().is_removed());
        assert_eq!(fixture.drops(&Item::PAINTING), 0);
        fixture.finish().await;
    }
}

async fn reload(fixture: &Fixture) {
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_reload_preserves_even_painting_and_frame_nbt() {
    let fixture = Fixture::new();
    for x in 4..=5 {
        for y in 64..=65 {
            fixture.set_block(BlockPos::new(x, y, 8), &Block::STONE);
        }
    }
    let painting = fixture.place(
        &Item::PAINTING,
        BlockPos::new(4, 64, 8),
        BlockDirection::South,
        Some(PaintingVariant::Bust),
    );
    let center = painting.get_entity().pos.load();
    let painting_id = painting.get_entity().entity_uuid;
    let mut frames = Vec::new();
    for (x, item, fixed, chance) in [
        (8, &Item::ITEM_FRAME, false, 1.0),
        (11, &Item::GLOW_ITEM_FRAME, true, 0.0),
    ] {
        let pos = BlockPos::new(x, 64, 8);
        fixture.set_block(pos, &Block::STONE);
        let frame = fixture.place(item, pos, BlockDirection::South, None);
        let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
        let mut stack = ItemStack::new(1, &Item::DIAMOND);
        stack.set_data_component(pumpkin_data::data_component_impl::CustomNameImpl {
            name: pumpkin_util::text::TextComponent::text("saved frame content"),
        });
        concrete.set_item(stack, false);
        concrete.set_rotation(6, false);
        concrete.set_drop_chance(chance);
        concrete.set_invisible(true);
        concrete.set_fixed(fixed);
        let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
        frame.write_custom_nbt(&mut nbt);
        frames.push((frame.get_entity().entity_uuid, pos, fixed, nbt));
    }
    fixture.tick(101);
    reload(&fixture).await;
    let loaded = fixture
        .world
        .get_entity_by_uuid(painting_id)
        .expect("even painting reloads");
    assert_eq!(
        loaded.get_entity().pos.load(),
        center,
        "saved center was shifted twice"
    );
    assert_eq!(
        loaded
            .cast_any()
            .downcast_ref::<super::painting::PaintingEntity>()
            .unwrap()
            .variant(),
        PaintingVariant::Bust
    );
    for (id, pos, fixed, expected) in &frames {
        let loaded = fixture
            .world
            .get_entity_by_uuid(*id)
            .expect("filled frame reloads");
        let mut actual = pumpkin_nbt::compound::NbtCompound::new();
        loaded.write_custom_nbt(&mut actual);
        assert_eq!(actual, *expected, "frame saved tags changed meaning");
        let concrete = loaded.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
        assert_eq!(concrete.get_item().item, &Item::DIAMOND);
        fixture.set_block(*pos, &Block::AIR);
        assert_eq!(concrete.is_fixed(), *fixed);
    }
    fixture.tick(101);
    assert!(!loaded.get_entity().is_removed());
    assert!(fixture.world.get_entity_by_uuid(frames[0].0).is_none());
    assert!(fixture.world.get_entity_by_uuid(frames[1].0).is_some());
    assert_eq!(fixture.drops(&Item::DIAMOND), 1);
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    assert_eq!(fixture.drops(&Item::GLOW_ITEM_FRAME), 0);
    tokio::time::timeout(
        Duration::from_secs(30),
        fixture
            .world
            .level
            .mark_chunks_as_not_watched([Vector2::new(0, 0)]),
    )
    .await
    .unwrap();
    reload(&fixture).await;
    assert!(
        fixture.world.get_entity_by_uuid(frames[0].0).is_none(),
        "broken frame resurrected"
    );
    assert!(fixture.world.get_entity_by_uuid(frames[1].0).is_some());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_missing_neighbor_defers_support_check() {
    let fixture = Fixture::new();
    fixture
        .player
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 12.5));
    let support = BlockPos::new(0, 64, 8);
    fixture.set_block(support, &Block::STONE);
    let frame = fixture.place(&Item::ITEM_FRAME, support, BlockDirection::South, None);
    fixture.set_block(support, &Block::AIR);
    fixture.tick(101);
    assert!(
        !frame.get_entity().is_removed(),
        "missing neighbor must postpone support validation"
    );
    let mut neighbor =
        crate::world::spawn_test_support::proto(&pumpkin_data::biome::Biome::PLAINS, &Block::STONE);
    neighbor.x = -1;
    crate::world::spawn_test_support::publish(&fixture.world, neighbor);
    fixture.tick(101);
    assert!(
        frame.get_entity().is_removed(),
        "loaded neighbor must permit support validation"
    );
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_opposing_and_perpendicular_map_frames_coexist() {
    let fixture = Fixture::new();
    let mut frames = Vec::new();
    for (support, face, item) in [
        (
            BlockPos::new(8, 64, 8),
            BlockDirection::South,
            &Item::ITEM_FRAME,
        ),
        (
            BlockPos::new(8, 64, 10),
            BlockDirection::North,
            &Item::ITEM_FRAME,
        ),
        (
            BlockPos::new(7, 64, 9),
            BlockDirection::East,
            &Item::GLOW_ITEM_FRAME,
        ),
    ] {
        fixture.set_block(support, &Block::STONE);
        let frame = fixture.place(item, support, face, None);
        let mut map = ItemStack::new(1, &Item::FILLED_MAP);
        map.set_data_component(pumpkin_data::data_component_impl::MapIdImpl { id: 7 });
        frame
            .cast_any()
            .downcast_ref::<ItemFrameEntity>()
            .unwrap()
            .set_item(map, false);
        frames.push(frame);
    }
    fixture.tick(202);
    for frame in frames {
        assert!(
            !frame.get_entity().is_removed(),
            "crossing frame directions must coexist"
        );
        assert_eq!(
            frame
                .cast_any()
                .downcast_ref::<ItemFrameEntity>()
                .unwrap()
                .get_item()
                .item,
            &Item::FILLED_MAP
        );
    }
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 0);
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
    assert_eq!(fixture.drops(&Item::GLOW_ITEM_FRAME), 0);
    fixture.finish().await;
}

fn command(fixture: &Fixture, input: &str) {
    fixture
        .server
        .worlds
        .store(Arc::new(vec![fixture.world.clone()]));
    fixture
        .player
        .player
        .permission_lvl
        .store(pumpkin_util::PermissionLvl::Four);
    fixture
        .server
        .command_dispatcher
        .load()
        .execute_input(
            input,
            &fixture.player.player.get_command_source(&fixture.server),
        )
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_teleported_fixed_frame_still_blocks_overlap() {
    let fixture = Fixture::new();
    let mut frames = Vec::new();
    for x in [8, 6] {
        let support = BlockPos::new(x, 64, 8);
        fixture.set_block(support, &Block::STONE);
        frames.push(fixture.place(&Item::ITEM_FRAME, support, BlockDirection::South, None));
    }
    let fixed = frames[1]
        .cast_any()
        .downcast_ref::<ItemFrameEntity>()
        .unwrap();
    fixed.set_fixed(true);
    let mut map = ItemStack::new(1, &Item::FILLED_MAP);
    map.set_data_component(pumpkin_data::data_component_impl::MapIdImpl { id: 7 });
    fixed.set_item(map, false);
    fixture.tick(101);
    assert!(!frames[0].get_entity().is_removed());
    command(
        &fixture,
        &format!("tp {} 8.5 64.5 9.5", frames[1].get_entity().entity_uuid),
    );
    fixture.tick(101);
    assert!(
        frames[0].get_entity().is_removed(),
        "teleport's generic cached box hid an overlapping Fixed frame"
    );
    assert!(!frames[1].get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g19_review_live_painting_data_refreshes_bounds_and_metadata() {
    use pumpkin_protocol::{VarInt, java::client::play::Metadata};
    let fixture = Fixture::new();
    for x in 6..=11 {
        for y in 62..=67 {
            fixture.set_block(BlockPos::new(x, y, 8), &Block::STONE);
        }
    }
    let painting = fixture.place(
        &Item::PAINTING,
        BlockPos::new(8, 65, 8),
        BlockDirection::South,
        Some(PaintingVariant::Alban),
    );
    let center = painting.get_entity().pos.load();
    painting.get_entity().synched_data.clear_dirty_for_test();
    command(
        &fixture,
        &format!(
            "data merge entity {} {{variant:\"minecraft:bust\"}}",
            painting.get_entity().entity_uuid
        ),
    );
    assert_eq!(
        painting.get_entity().pos.load(),
        center,
        "NBT Pos must keep its saved-center meaning"
    );
    let bounds = painting.get_entity().bounding_box.load();
    assert_eq!(bounds.max.x - bounds.min.x, 2.0);
    assert_eq!(bounds.max.y - bounds.min.y, 2.0);
    let mut expected = Vec::new();
    Metadata::new(
        pumpkin_data::tracked_data::painting::DATA_PAINTING_VARIANT_ID,
        VarInt(PaintingVariant::Bust.id() as i32 + 1),
    )
    .write(&mut expected, &pumpkin_data::packet::CURRENT_MC_VERSION)
    .unwrap();
    let dirty = painting
        .get_entity()
        .synched_data
        .pack_dirty_for_version(&pumpkin_data::packet::CURRENT_MC_VERSION)
        .expect("live data edit publishes metadata");
    assert!(
        dirty.windows(expected.len()).any(|bytes| bytes == expected),
        "live NBT variant was not synchronized"
    );
    fixture.tick(101);
    assert!(!painting.get_entity().is_removed());
    fixture.finish().await;
}
