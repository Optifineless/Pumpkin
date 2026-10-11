use super::*;
use crate::{
    entity::death_test_world::DeathTestWorld,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{biome::Biome, data_component_impl::BannerPatternsImpl};
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_world::world::BlockFlags;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wall_banner_support_removal_preserves_patterned_drop() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let position = BlockPos::new(8, 64, 8);
    let support = BlockPos::new(8, 64, 7);
    world.set_block_state(
        &support,
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let mut props = WhiteWallBannerProperties::default(&Block::WHITE_WALL_BANNER);
    props.facing = pumpkin_data::block_properties::HorizontalFacing::South;
    world.set_block_state(
        &position,
        props.to_state_id(&Block::WHITE_WALL_BANNER),
        BlockFlags::FORCE_STATE,
    );
    let banner = BannerBlockEntity::new(position);
    let mut layer = NbtCompound::new();
    layer.put_string("pattern", "minecraft:stripe_bottom".to_owned());
    layer.put_string("color", "red".to_owned());
    *banner.patterns.lock().unwrap() = Some(vec![NbtTag::Compound(layer)]);
    world.add_block_entity(Arc::new(banner));
    // Creative removal skips the support's own drops, while dependent banners still drop.
    world.break_block(
        &support,
        None,
        BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
    );
    assert!(world.get_block_state(&position).is_air());
    let entities = world.entities.load_full();
    let drops: Vec<_> = entities
        .iter()
        .filter_map(|e| e.get_item_entity())
        .map(|e| e.get_item_stack().lock().unwrap().clone())
        .collect();
    assert_eq!(drops.len(), 1);
    assert_eq!(drops[0].item, &pumpkin_data::item::Item::WHITE_BANNER);
    let patterns = drops[0].get_data_component::<BannerPatternsImpl>().unwrap();
    assert_eq!(patterns.layers.len(), 1);
    assert_eq!(
        patterns.layers[0].pattern,
        "minecraft:stripe_bottom".to_owned()
    );
    fixture.server.shutdown().await;
    crate::server::fixture_lifecycle::finish().await;
}

#[pumpkin_macros::pumpkin_block("minecraft:stone")]
struct RestoreSupport {
    restored: [std::sync::atomic::AtomicBool; 2],
}
impl crate::block::BlockBehaviour for RestoreSupport {
    fn on_neighbor_update(&self, args: crate::block::OnNeighborUpdateArgs<'_>) {
        let supports = [BlockPos::new(8, 63, 8), BlockPos::new(8, 64, 7)];
        let Some(index) = supports
            .iter()
            .position(|position| position == args.position)
        else {
            return;
        };
        if self.restored[index].swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        // Queue a removal shape, then restore support while the outer cascade is still running.
        args.world
            .set_block_state(args.position, BlockStateId::AIR, BlockFlags::NOTIFY_ALL);
        args.world.set_block_state(
            args.position,
            Block::STONE.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_support_remove_restore_keeps_supported_banners() {
    let fixture = DeathTestWorld::with_neighbor_limit(100).await;
    let original = fixture.world();
    let mut registry = crate::block::registry::default_registry();
    Arc::get_mut(&mut registry)
        .unwrap()
        .register(RestoreSupport {
            restored: std::array::from_fn(|_| std::sync::atomic::AtomicBool::new(false)),
        });
    let world = Arc::new(crate::world::World::load(
        original.level.clone(),
        original.level_info.clone(),
        original.dimension.clone(),
        registry,
        Arc::downgrade(&fixture.server),
    ));
    crate::server::fixture_lifecycle::track_world(&world);
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    for wall in [false, true] {
        let position = BlockPos::new(8, 64, 8);
        let support = if wall {
            BlockPos::new(8, 64, 7)
        } else {
            position.down()
        };
        let state = if wall {
            let mut props = WhiteWallBannerProperties::default(&Block::WHITE_WALL_BANNER);
            props.facing = pumpkin_data::block_properties::HorizontalFacing::South;
            props.to_state_id(&Block::WHITE_WALL_BANNER)
        } else {
            Block::WHITE_BANNER.default_state.id
        };
        world.set_block_state(
            &support,
            Block::STONE.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
        world.set_block_state(
            &position,
            state,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
        world.update_neighbor(&support, &Block::STONE);
        assert_eq!(world.get_block_state_id(&position), state);
        assert!(
            world
                .entities
                .load()
                .iter()
                .all(|entity| entity.get_item_entity().is_none())
        );
        world.set_block_state(
            &position,
            BlockStateId::AIR,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }
    fixture.server.shutdown().await;
    crate::server::fixture_lifecycle::finish().await;
}
