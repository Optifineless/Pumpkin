use super::*;
use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{Block, data_component_impl::ContainerImpl, item::Item, item_stack::ItemStack};
use pumpkin_protocol::{
    VarInt,
    codec::item_stack_seralizer::OptionalItemStackHash,
    java::server::play::{SClickSlot, SlotActionType},
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::world::BlockFlags;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deep_review_mining_open_shulker_rejects_stale_clicks_and_preserves_one_copy() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let viewer = TestPlayer::new(&world).player;
    let miner = TestPlayer::new(&world).player;
    world
        .players
        .store(Arc::new(vec![viewer.clone(), miner.clone()]));
    miner
        .permission_lvl
        .store(pumpkin_util::permission::PermissionLvl::Four);
    let pos = BlockPos::new(8, 64, 8);
    let (entity, menu) = open_box(&server, &world, &viewer, pos);
    assert!(menu.lock().unwrap().can_use(viewer.as_ref()));
    viewer
        .get_entity()
        .set_pos(pos.to_centered_f64() + pumpkin_util::math::vector3::Vector3::new(32.0, 0.0, 0.0));
    assert!(!menu.lock().unwrap().can_use(viewer.as_ref()));
    viewer.get_entity().set_pos(pos.to_centered_f64());
    *viewer.current_screen_handler.lock().unwrap() = menu.clone();
    assert!(
        world
            .break_block(&pos, Some(&miner), BlockFlags::NOTIFY_ALL)
            .is_some()
    );
    let drops: Vec<_> = world
        .entities
        .load()
        .iter()
        .filter_map(|e| e.get_item_entity())
        .map(|e| e.get_item_stack().lock().unwrap().clone())
        .collect();
    assert_eq!(drops.len(), 1);
    assert_eq!(
        drops[0]
            .get_data_component::<ContainerImpl>()
            .unwrap()
            .items[0]
            .1
            .item_count,
        17
    );
    viewer.on_slot_click(
        SClickSlot {
            sync_id: VarInt(1),
            revision: VarInt(0),
            slot: 0,
            button: 0,
            mode: SlotActionType::QuickMove,
            length_of_array: VarInt(0),
            array_of_changed_slots: Vec::new(),
            carried_item: OptionalItemStackHash(None),
        },
        &server,
    );
    assert!(viewer.inventory.is_empty());
    assert!(!menu.lock().unwrap().can_use(viewer.as_ref()));
    assert_eq!(entity.get_stack(0).item_count, 17);
    assert!(
        viewer
            .current_screen_handler
            .lock()
            .unwrap()
            .lock()
            .unwrap()
            .window_type()
            .is_none()
    );
    // Replacing the block at the same position cannot revive the old menu.
    world.add_block_entity(Arc::new(ShulkerBoxBlockEntity::new(pos)));
    assert!(!menu.lock().unwrap().can_use(viewer.as_ref()));
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

fn open_box(
    server: &Arc<crate::server::Server>,
    world: &Arc<crate::world::World>,
    viewer: &Arc<crate::entity::player::Player>,
    pos: BlockPos,
) -> (Arc<ShulkerBoxBlockEntity>, SharedScreenHandler) {
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(8, 64, 8, Block::SHULKER_BOX.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let entity = Arc::new(ShulkerBoxBlockEntity::new(pos));
    entity.set_stack(0, ItemStack::new(17, &Item::DIAMOND));
    world.add_block_entity(entity.clone());
    viewer.get_entity().set_pos(pos.to_centered_f64());
    let factory = ShulkerBoxBlock
        .get_screen_handler_factory(GetScreenHandlerFactoryArgs {
            server,
            world,
            block: &Block::SHULKER_BOX,
            position: &pos,
            player: viewer,
        })
        .unwrap();
    let menu = factory
        .create_screen_handler(1, &viewer.inventory, viewer.as_ref())
        .unwrap();
    (entity, menu)
}
