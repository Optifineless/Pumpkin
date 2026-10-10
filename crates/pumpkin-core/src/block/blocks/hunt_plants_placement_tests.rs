use super::{Fixture, packet};
use crate::{
    block::{BlockBehaviour, GetStateForNeighborUpdateArgs, OnPlaceArgs},
    entity::EntityBase,
    plugin::{
        EventHandler, EventPriority, api::events::block::block_place::BlockPlaceEvent,
        block::block_can_build::BlockCanBuildEvent,
    },
    server::Server,
};
use futures::future::BoxFuture;
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    block_properties::{
        DriedGhastLikeProperties, GlowLichenLikeProperties, HorizontalFacing,
        ShelfMushroomLikeProperties, SlabType, TurtleEggLikeProperties,
        WhiteWoolSlabLikeProperties,
    },
    fluid::Fluid,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lily_pad_on_water_survives_neighbor_update() {
    let f = Fixture::new();
    let pos = BlockPos::new(8, 64, 8);
    for support in [
        Block::WATER.default_state.id,
        Block::ICE.default_state.id,
        Block::FROSTED_ICE.default_state.id,
    ] {
        f.set(pos.down(), support);
        f.set(pos, Block::LILY_PAD.default_state.id);
        f.world.set_block_state(
            &pos.up(),
            Block::STONE.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        assert_eq!(f.world.get_block(&pos), &Block::LILY_PAD);
        f.world
            .set_block_state(&pos.up(), BlockStateId::AIR, BlockFlags::NOTIFY_ALL);
        assert_eq!(f.world.get_block(&pos), &Block::LILY_PAD);
    }
    // LilyPadBlock.mayPlaceOn also recognizes source water contained in another block.
    let slab = Block::OAK_SLAB
        .set_waterlogged(Block::OAK_SLAB.default_state.id, true)
        .unwrap();
    f.set(pos.down(), slab);
    f.set(pos, Block::LILY_PAD.default_state.id);
    f.world.set_block_state(
        &pos.up(),
        Block::STONE.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    assert_eq!(f.world.get_block(&pos), &Block::LILY_PAD);
    // A nonempty fluid at the pad position prevents survival, including contained water or lava.
    let behavior = f
        .world
        .block_registry
        .get_pumpkin_block(Block::LILY_PAD.id)
        .unwrap();
    for above in [slab, Block::LAVA.default_state.id] {
        f.set(pos, above);
        assert_eq!(
            behavior.get_state_for_neighbor_update(GetStateForNeighborUpdateArgs {
                world: &f.world,
                block: &Block::LILY_PAD,
                state_id: Block::LILY_PAD.default_state.id,
                position: &pos,
                direction: BlockDirection::Up,
                neighbor_position: &pos.up(),
                neighbor_state_id: BlockStateId::AIR,
            }),
            BlockStateId::AIR
        );
    }
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mining_turtle_egg_cluster_removes_one_egg() {
    let f = Fixture::new();
    let pos = BlockPos::new(8, 64, 8);
    let mut props = TurtleEggLikeProperties::default(&Block::TURTLE_EGG);
    props.eggs = 4;
    props.hatch = 1;
    f.set(pos, props.to_state_id(&Block::TURTLE_EGG));
    for remaining in (1..=3).rev() {
        f.mine(pos);
        assert_eq!(f.world.get_block(&pos), &Block::TURTLE_EGG);
        let props = TurtleEggLikeProperties::from_state_id(f.world.get_block_state_id(&pos));
        assert_eq!(props.eggs, remaining);
        assert_eq!(props.hatch, 1);
    }
    f.mine(pos);
    assert!(f.world.get_block_state(&pos).is_air());
    props.eggs = 4;
    f.set(pos, props.to_state_id(&Block::TURTLE_EGG));
    f.player.player.gamemode.store(GameMode::Creative);
    f.mine(pos);
    assert!(f.world.get_block_state(&pos).is_air());
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn breaking_sniffer_egg_drops_exactly_one_in_survival_and_none_in_creative() {
    let mut f = Fixture::new();
    let pos = BlockPos::new(8, 64, 8);
    f.set(pos, Block::SNIFFER_EGG.default_state.id);
    f.player.take_packets();
    f.mine(pos);
    assert_eq!(f.item_count(&Item::SNIFFER_EGG), 1);
    assert!(
        f.take_sounds().is_empty(),
        "SnifferEggBlock has no extra crack sound on a player break"
    );
    f.set(pos, Block::SNIFFER_EGG.default_state.id);
    f.player.player.gamemode.store(GameMode::Creative);
    f.mine(pos);
    assert_eq!(f.item_count(&Item::SNIFFER_EGG), 1);
    assert!(f.world.get_block_state(&pos).is_air());
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shelf_mushroom_attaches_to_a_stone_wall() {
    let f = Fixture::new();
    let pos = BlockPos::new(8, 64, 8);
    f.set(pos.down(), BlockStateId::AIR);
    f.player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(5, &Item::SHELF_MUSHROOM));
    for facing in HorizontalFacing::all() {
        let support = pos.offset(facing.opposite().to_offset());
        f.set(support, Block::STONE.default_state.id);
        f.use_block(
            support,
            pumpkin_data::HorizontalFacingExt::to_block_direction(&facing),
        );
        assert_eq!(f.world.get_block(&pos), &Block::SHELF_MUSHROOM);
        assert_eq!(
            ShelfMushroomLikeProperties::from_state_id(f.world.get_block_state_id(&pos)).facing,
            facing
        );
        f.set(pos, BlockStateId::AIR);
        f.set(support, BlockStateId::AIR);
    }
    assert_eq!(f.player.player.inventory().held_item().item_count, 1);
    f.use_block(pos, BlockDirection::North);
    assert!(f.world.get_block_state(&pos).is_air());
    assert_eq!(f.player.player.inventory().held_item().item_count, 1);
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snow_on_unsupported_surface_is_refused_and_not_consumed() {
    let mut f = Fixture::new();
    let support = BlockPos::new(8, 63, 8);
    f.set(support, Block::ICE.default_state.id);
    f.player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::SNOW));
    f.player.take_packets();
    f.use_block(support, BlockDirection::Up);
    assert!(f.world.get_block_state(&support.up()).is_air());
    assert_eq!(f.player.player.inventory().held_item().item_count, 2);
    assert_eq!(
        f.player.player.stats.lock().unwrap().get(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(Item::SNOW.id)
        ),
        0
    );
    f.set(support, Block::STONE.default_state.id);
    f.use_block(support, BlockDirection::Up);
    assert_eq!(f.world.get_block(&support.up()), &Block::SNOW);
    assert_eq!(f.player.player.inventory().held_item().item_count, 1);
    assert!(f.world.level.shutdown().await.is_ok());
}

// Exercise the real banner resolver's null result even if the pre-placement gate admits it.
#[pumpkin_macros::pumpkin_block("minecraft:white_banner")]
struct AdmittedBanner;
impl BlockBehaviour for AdmittedBanner {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        crate::block::blocks::banners::BannerBlock.on_place(args)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn banner_without_support_is_refused_and_not_consumed() {
    let mut registry = crate::block::registry::default_registry();
    Arc::get_mut(&mut registry)
        .unwrap()
        .register(AdmittedBanner);
    let f = Fixture::with_registry(registry);
    let pos = BlockPos::new(8, 65, 8);
    f.player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::WHITE_BANNER));
    f.use_block(pos, BlockDirection::Up);
    assert!(f.world.get_block_state(&pos).is_air());
    assert!(f.world.get_block_state(&pos.up()).is_air());
    assert_eq!(f.player.player.inventory().held_item().item_count, 2);
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adding_a_multiface_face_consumes_one_item() {
    let f = Fixture::new();
    let pos = BlockPos::new(8, 64, 8);
    let wall = pos.offset(Vector3::new(-1, 0, 0));
    f.set(wall, Block::STONE.default_state.id);
    for (block, item) in [
        (&Block::GLOW_LICHEN, &Item::GLOW_LICHEN),
        (&Block::SCULK_VEIN, &Item::SCULK_VEIN),
        (&Block::RESIN_CLUMP, &Item::RESIN_CLUMP),
    ] {
        for mode in [GameMode::Survival, GameMode::Creative] {
            f.player.player.gamemode.store(mode);
            f.player
                .player
                .inventory()
                .set_stack(0, ItemStack::new(3, item));
            let mut props = GlowLichenLikeProperties::default(block);
            props.down = true;
            props.waterlogged = true;
            f.set(pos, props.to_state_id(block));
            f.use_block(pos, BlockDirection::Up);
            props = GlowLichenLikeProperties::from_state_id(f.world.get_block_state_id(&pos));
            assert!(props.down && props.west && props.waterlogged);
            let expected = if mode == GameMode::Creative { 3 } else { 2 };
            assert_eq!(f.player.player.inventory().held_item().item_count, expected);
            // All other directions lack support. Repeating the occupied face changes nothing.
            let state = f.world.get_block_state_id(&pos);
            f.use_block(pos, BlockDirection::Up);
            assert_eq!(f.world.get_block_state_id(&pos), state);
            assert_eq!(f.player.player.inventory().held_item().item_count, expected);
        }
    }
    // Replacing the clicked block follows the view direction before the clicked face.
    f.set(
        pos.offset(Vector3::new(0, 0, -1)),
        Block::STONE.default_state.id,
    );
    f.player.player.gamemode.store(GameMode::Survival);
    f.player.player.get_entity().set_rotation(180.0, 0.0);
    f.player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::GLOW_LICHEN));
    let mut props = GlowLichenLikeProperties::default(&Block::GLOW_LICHEN);
    props.down = true;
    f.set(pos, props.to_state_id(&Block::GLOW_LICHEN));
    f.use_block(pos, BlockDirection::East);
    props = GlowLichenLikeProperties::from_state_id(f.world.get_block_state_id(&pos));
    assert!(props.north && !props.west);
    assert_eq!(f.player.player.inventory().held_item().item_count, 1);
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_multiface_addition_keeps_clicked_target_and_item() {
    let f = Fixture::new();
    let pos = BlockPos::new(8, 64, 8);
    let fallback = pos.offset(BlockDirection::East.to_offset());
    f.set(pos.down(), BlockStateId::AIR);
    f.set(fallback.down(), BlockStateId::AIR);
    f.set(
        pos.offset(BlockDirection::West.to_offset()),
        Block::STONE.default_state.id,
    );
    // A redirected placement could attach to this ceiling and consume an item.
    f.set(fallback.up(), Block::STONE.default_state.id);
    for (block, item) in [
        (&Block::GLOW_LICHEN, &Item::GLOW_LICHEN),
        (&Block::SCULK_VEIN, &Item::SCULK_VEIN),
        (&Block::RESIN_CLUMP, &Item::RESIN_CLUMP),
    ] {
        let mut props = GlowLichenLikeProperties::default(block);
        props.west = true;
        let state = props.to_state_id(block);
        f.set(pos, state);
        f.player
            .player
            .inventory()
            .set_stack(0, ItemStack::new(2, item));
        f.use_block(pos, BlockDirection::East);
        assert_eq!(f.world.get_block_state_id(&pos), state);
        assert!(f.world.get_block_state(&fallback).is_air());
        assert_eq!(f.player.player.inventory().held_item().item_count, 2);
    }
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multiface_attaches_to_top_and_double_slab_faces() {
    let f = Fixture::new();
    let pos = BlockPos::new(8, 64, 8);
    f.set(pos.down(), BlockStateId::AIR);
    for (block, item) in [
        (&Block::GLOW_LICHEN, &Item::GLOW_LICHEN),
        (&Block::SCULK_VEIN, &Item::SCULK_VEIN),
        (&Block::RESIN_CLUMP, &Item::RESIN_CLUMP),
    ] {
        for (slab_type, attachment) in [
            (SlabType::Top, BlockDirection::Down),
            (SlabType::Double, BlockDirection::West),
        ] {
            let support = pos.offset(attachment.to_offset());
            let mut slab = WhiteWoolSlabLikeProperties::default(&Block::OAK_SLAB);
            slab.r#type = slab_type;
            f.set(support, slab.to_state_id(&Block::OAK_SLAB));
            f.player
                .player
                .inventory()
                .set_stack(0, ItemStack::new(2, item));
            f.use_block(support, attachment.opposite());
            assert_eq!(f.world.get_block(&pos), block);
            let props = GlowLichenLikeProperties::from_state_id(f.world.get_block_state_id(&pos));
            assert!(if attachment == BlockDirection::Down {
                props.down
            } else {
                props.west
            });
            assert_eq!(f.player.player.inventory().held_item().item_count, 1);
            // A neighbour update retains the face against the actual supporting slab state.
            f.world.set_block_state(
                &pos.up(),
                Block::STONE.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
            assert_eq!(f.world.get_block(&pos), block);
            f.set(pos.up(), BlockStateId::AIR);
            // A bottom slab has no full top or west face: the existing attachment must pop.
            slab.r#type = SlabType::Bottom;
            f.world.set_block_state(
                &support,
                slab.to_state_id(&Block::OAK_SLAB),
                BlockFlags::NOTIFY_ALL,
            );
            assert!(f.world.get_block_state(&pos).is_air());
            f.set(support, BlockStateId::AIR);
        }
    }
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dried_ghast_placement_waterlogging_and_neighbor_water_tick() {
    let mut f = Fixture::new();
    let pos = BlockPos::new(8, 64, 8);
    f.set(pos, Block::WATER.default_state.id);
    f.player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::DRIED_GHAST));
    f.player.player.get_entity().set_rotation(90.0, 0.0);
    f.use_block(pos.down(), BlockDirection::Up);
    let mut props = DriedGhastLikeProperties::from_state_id(f.world.get_block_state_id(&pos));
    assert!(props.waterlogged);
    assert_eq!(props.facing, HorizontalFacing::East);
    assert_eq!(f.player.player.inventory().held_item().item_count, 1);
    assert!(
        f.take_sounds()
            .contains(&(pumpkin_data::sound::Sound::BlockDriedGhastPlaceInWater as u16))
    );
    // Use the real bucket container path, then observe updateShape's pending water tick.
    crate::item::items::bucket::try_pickup_fluid_at(&f.world, pos);
    props = DriedGhastLikeProperties::from_state_id(f.world.get_block_state_id(&pos));
    assert!(!props.waterlogged);
    assert_eq!(
        crate::item::items::bucket::empty_bucket_at(&f.world, &Item::WATER_BUCKET, pos),
        Some(pos)
    );
    props = DriedGhastLikeProperties::from_state_id(f.world.get_block_state_id(&pos));
    assert!(props.waterlogged);
    assert!(
        f.take_sounds()
            .contains(&(pumpkin_data::sound::Sound::BlockDriedGhastPlaceInWater as u16))
    );
    let chunk = f
        .world
        .level
        .loaded_chunks
        .get(&pos.chunk_position())
        .unwrap()
        .clone();
    chunk
        .fluid_ticks
        .clear_area(&pos, &pos.offset(Vector3::new(1, 1, 1)));
    let behavior = f
        .world
        .block_registry
        .get_pumpkin_block(Block::DRIED_GHAST.id)
        .unwrap();
    let state = f.world.get_block_state_id(&pos);
    assert_eq!(
        behavior.get_state_for_neighbor_update(GetStateForNeighborUpdateArgs {
            world: &f.world,
            block: &Block::DRIED_GHAST,
            state_id: state,
            position: &pos,
            direction: BlockDirection::East,
            neighbor_position: &pos.offset(Vector3::new(1, 0, 0)),
            neighbor_state_id: BlockStateId::AIR,
        }),
        state
    );
    assert!(f.world.level.is_fluid_tick_scheduled(&pos, &Fluid::WATER));
    assert!(f.world.level.shutdown().await.is_ok());
}

#[derive(Default)]
struct PlacementEvents(AtomicUsize);
impl EventHandler<BlockCanBuildEvent> for PlacementEvents {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut BlockCanBuildEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.0.fetch_add(1, Relaxed);
        })
    }
}
impl EventHandler<BlockPlaceEvent> for PlacementEvents {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut BlockPlaceEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.0.fetch_add(1, Relaxed);
        })
    }
}
#[pumpkin_macros::pumpkin_block("minecraft:stone")]
struct PlacementState(BlockStateId);
impl BlockBehaviour for PlacementState {
    fn on_place(&self, _: OnPlaceArgs<'_>) -> BlockStateId {
        self.0
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn air_placement_guard_precedes_side_effects_and_allows_other_air_states() {
    let mut f = Fixture::new();
    let mut registry = crate::block::registry::default_registry();
    Arc::get_mut(&mut registry)
        .unwrap()
        .register(PlacementState(BlockStateId::AIR));
    let world = Arc::new(crate::world::World::load(
        f.world.level.clone(),
        f.world.level_info.clone(),
        f.world.dimension.clone(),
        registry,
        Arc::downgrade(&f.server),
    ));
    let events = Arc::new(PlacementEvents::default());
    f.server.plugin_manager.register::<BlockCanBuildEvent, _>(
        events.clone(),
        EventPriority::Normal,
        true,
    );
    f.server.plugin_manager.register::<BlockPlaceEvent, _>(
        events.clone(),
        EventPriority::Normal,
        true,
    );
    let pos = BlockPos::new(8, 64, 8);
    f.player.take_packets();
    let request = packet(pos.down(), BlockDirection::Up);
    assert!(
        world
            .block_registry
            .place_block(
                &f.player.player,
                &Block::STONE,
                &f.server,
                &request,
                pos.down(),
                BlockDirection::Up
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(events.0.load(Relaxed), 0);
    assert!(f.player.take_packets().is_empty());
    assert_eq!(world.get_block_state_id(&pos), BlockStateId::AIR);
    // AIR is the existing null-state sentinel; cave/void air are legitimate resolved states.
    let mut registry = crate::block::registry::default_registry();
    Arc::get_mut(&mut registry)
        .unwrap()
        .register(PlacementState(Block::CAVE_AIR.default_state.id));
    let world = Arc::new(crate::world::World::load(
        f.world.level.clone(),
        f.world.level_info.clone(),
        f.world.dimension.clone(),
        registry,
        Arc::downgrade(&f.server),
    ));
    assert!(
        world
            .block_registry
            .place_block(
                &f.player.player,
                &Block::STONE,
                &f.server,
                &request,
                pos.down(),
                BlockDirection::Up
            )
            .unwrap()
            .is_some()
    );
    assert_eq!(
        world.get_block_state_id(&pos),
        Block::CAVE_AIR.default_state.id
    );
    assert!(f.world.level.shutdown().await.is_ok());
}
