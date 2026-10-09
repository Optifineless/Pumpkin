use super::{
    BlockEntity, barrel::BarrelBlockEntity, chest::ChestBlockEntity,
    ender_chest::EnderChestBlockEntity, shulker_box::ShulkerBoxBlockEntity,
    trapped_chest::TrappedChestBlockEntity,
};
use crate::{
    entity::death_test_world::DeathTestWorld,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::world::generic_game::GenericGameEvent,
    },
    server::Server,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{Block, biome::Biome};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, Mutex};

struct Events(Arc<Mutex<Vec<String>>>);
impl EventHandler<GenericGameEvent> for Events {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key.ends_with("container_open")
            || event.event_key.ends_with("container_close")
        {
            self.0.lock().unwrap().push(event.event_key.clone());
        }
        Box::pin(async {})
    }
}

fn change_viewers(entity: &Arc<dyn BlockEntity>, opening: bool) {
    if let Some(chest) = entity.as_any().downcast_ref::<EnderChestBlockEntity>() {
        let tracker = chest.get_tracker();
        if opening {
            tracker.open_container();
        } else {
            tracker.close_container();
        }
    } else {
        let inventory = entity.clone().get_inventory().unwrap();
        if opening {
            inventory.on_open();
        } else {
            inventory.on_close();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn container_events_follow_zero_viewer_boundaries() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let events = Arc::new(Mutex::new(Vec::new()));
    fixture
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(
            Arc::new(Events(events.clone())),
            EventPriority::Normal,
            true,
        );
    let cases: [(Block, Arc<dyn BlockEntity>); 5] = [
        (
            Block::BARREL,
            Arc::new(BarrelBlockEntity::new(BlockPos::new(2, 64, 2))),
        ),
        (
            Block::CHEST,
            Arc::new(ChestBlockEntity::new(BlockPos::new(4, 64, 2))),
        ),
        (
            Block::TRAPPED_CHEST,
            Arc::new(TrappedChestBlockEntity::new(BlockPos::new(6, 64, 2))),
        ),
        (
            Block::ENDER_CHEST,
            Arc::new(EnderChestBlockEntity::new(BlockPos::new(8, 64, 2))),
        ),
        (
            Block::SHULKER_BOX,
            Arc::new(ShulkerBoxBlockEntity::new(BlockPos::new(10, 64, 2))),
        ),
    ];
    for (block, entity) in cases {
        let position = entity.get_position();
        world.set_block_state(&position, block.default_state.id, BlockFlags::FORCE_STATE);
        world.add_block_entity(entity.clone());
        events.lock().unwrap().clear();
        change_viewers(&entity, true);
        entity.tick(&world);
        change_viewers(&entity, true);
        entity.tick(&world);
        change_viewers(&entity, false);
        entity.tick(&world);
        assert_eq!(*events.lock().unwrap(), ["container_open"]);
        change_viewers(&entity, false);
        entity.tick(&world);
        assert_eq!(
            *events.lock().unwrap(),
            ["container_open", "container_close"]
        );
        entity.tick(&world);
        assert_eq!(events.lock().unwrap().len(), 2);
    }
    fixture.server.shutdown().await;
}
