use super::*;
use crate::entity::living::test_support::armor_test_world;
use pumpkin_data::{
    data_component_impl::{BannerPatternsImpl, CustomNameImpl},
    item::Item,
};
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_util::math::vector2::Vector2;

#[tokio::test]
async fn exploded_wall_torch_uses_standing_torch_loot() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let explosion = Explosion::new(4.0, Vector3::new(0.5, 64.5, 0.5), BlockInteraction::Destroy);
    let pos = BlockPos::new(0, 64, 0);
    let torch = explosion
        .prepare_block_drops(&world, &pos, Block::WALL_TORCH.default_state)
        .finish(&world, &pos, None);
    assert_eq!(torch.len(), 1);
    assert_eq!(torch[0].item, &Item::TORCH);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exploded_banner_keeps_patterns_and_custom_name() {
    let dir = tempfile::tempdir().unwrap();
    let server = crate::server::combat_test_support::server(dir.path());
    let world = crate::server::combat_test_support::world(&server, dir.path());
    crate::server::combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    let explosion = Explosion::new(1.0, Vector3::new(0.5, 64.5, 0.5), BlockInteraction::Destroy);
    let pos = BlockPos::new(0, 64, 0);
    world.set_block_state(
        &pos.down(),
        Block::BEDROCK.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    world.set_block_state(
        &pos,
        Block::WHITE_BANNER.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let banner = crate::block::entities::banner::BannerBlockEntity::new(pos);
    let mut pattern = NbtCompound::new();
    pattern.put_string("pattern", "minecraft:stripe_bottom".into());
    pattern.put_string("color", "red".into());
    *banner.patterns.lock().unwrap() = Some(vec![NbtTag::Compound(pattern)]);
    *banner.custom_name.lock().unwrap() = Some(r#"{"text":"Blast banner"}"#.into());
    world.add_block_entity(Arc::new(banner));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    server
        .plugin_manager
        .register::<crate::plugin::block::block_drop_item::BlockDropItemEvent, _>(
            Arc::new(ObserveExplodingBanner {
                position: pos,
                calls: calls.clone(),
            }),
            crate::plugin::EventPriority::Normal,
            true,
        );
    explosion.explode(&world);
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(world.get_block(&pos), &Block::AIR);
    let banner = world
        .entities
        .load()
        .iter()
        .filter_map(|entity| {
            let item = entity.get_item_entity()?;
            let stack = item.get_item_stack().lock().unwrap().clone();
            (stack.item == &Item::WHITE_BANNER).then_some(stack)
        })
        .collect::<Vec<_>>();
    assert_eq!(banner.len(), 1);
    let patterns = banner[0]
        .get_data_component::<BannerPatternsImpl>()
        .unwrap();
    assert_eq!(patterns.layers[0].pattern, "minecraft:stripe_bottom");
    assert_eq!(
        banner[0]
            .get_data_component::<CustomNameImpl>()
            .unwrap()
            .name
            .clone()
            .get_text(),
        "Blast banner"
    );
}

struct ObserveExplodingBanner {
    position: BlockPos,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl crate::plugin::EventHandler<crate::plugin::block::block_drop_item::BlockDropItemEvent>
    for ObserveExplodingBanner
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<crate::server::Server>,
        event: &'a mut crate::plugin::block::block_drop_item::BlockDropItemEvent,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        if event.block_pos == self.position {
            assert_eq!(
                event.world.get_block(&self.position),
                &Block::WHITE_BANNER,
                "explosion loot callbacks must inspect the original block before removal"
            );
            assert!(
                event.world.get_block_entity(&self.position).is_some(),
                "explosion loot callbacks must inspect the original block entity"
            );
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Box::pin(async {})
    }
}

#[tokio::test]
async fn explosion_rays_stop_at_unloaded_chunk_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    world.level.loaded_chunks.insert(
        Vector2::new(0, 0),
        pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
    );
    let explosion = Explosion::new(
        4.0,
        Vector3::new(15.5, 64.5, 8.5),
        BlockInteraction::Destroy,
    );
    let mut positions = FxHashSet::default();
    explosion.trace_block_ray(
        &world,
        &DefaultExplosionDamageCalculator,
        Vector3::new(1.0, 0.0, 0.0),
        &mut positions,
    );
    assert!(!positions.is_empty());
    assert!(positions.iter().all(|pos| pos.0.x < 16));
    assert!(!Explosion::ray_clear(
        &world,
        explosion.pos,
        Vector3::new(18.5, 64.5, 8.5)
    ));
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(1, 0);
    chunk.set_block_absolute_y(0, 64, 8, Block::STONE.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(1, 0), chunk);
    assert!(!Explosion::ray_clear(
        &world,
        explosion.pos,
        Vector3::new(18.5, 64.5, 8.5)
    ));
}
