use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::SeqCst},
};

use pumpkin_data::{Block, biome::Biome, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use crate::{
    block::{
        BlockBehaviour, OnScheduledTickArgs, blocks::redstone::dispenser::DispenserBlock,
        entities::dispenser::DispenserBlockEntity,
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        entity::{entity_spawn::EntitySpawnEvent, projectile_launch::ProjectileLaunchEvent},
    },
    projectile_item_event_tests::run,
    server::Server,
    world::spawn_test_support::{proto, publish},
};

struct Reject {
    dispenser: Arc<DispenserBlockEntity>,
    calls: Arc<AtomicUsize>,
}

impl Reject {
    fn check_unlocked(&self) {
        let owner = self.dispenser.slot_transaction(0);
        assert!(
            std::thread::scope(|scope| scope
                .spawn(move || owner.try_lock().is_some())
                .join()
                .unwrap()),
            "dispenser callback held its inventory owner"
        );
        self.calls.fetch_add(1, SeqCst);
    }
}

impl EventHandler<ProjectileLaunchEvent> for Reject {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut ProjectileLaunchEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(event.shooter_id, None);
        self.check_unlocked();
        event.cancelled = true;
        Box::pin(async {})
    }
}

impl EventHandler<EntitySpawnEvent> for Reject {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        self.check_unlocked();
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_dispenser_keeps_rejected_projectiles_including_fire_charge() {
    for veto in [None, Some(false), Some(true)] {
        run(|fixture| {
            let world = fixture.world();
            publish(&world, proto(&Biome::PLAINS, &Block::STONE));
            let position = BlockPos::new(8, 64, 8);
            world.set_block_state(
                &position,
                Block::DISPENSER.default_state.id,
                BlockFlags::FORCE_STATE,
            );
            let dispenser = Arc::new(DispenserBlockEntity::new(position));
            world.add_block_entity(dispenser.clone());
            let calls = Arc::new(AtomicUsize::new(0));
            if let Some(launch) = veto {
                let handler = Arc::new(Reject {
                    dispenser: dispenser.clone(),
                    calls: calls.clone(),
                });
                if launch {
                    fixture
                        .server
                        .plugin_manager
                        .register::<ProjectileLaunchEvent, _>(handler, EventPriority::Normal, true);
                } else {
                    fixture
                        .server
                        .plugin_manager
                        .register::<EntitySpawnEvent, _>(handler, EventPriority::Normal, true);
                }
            }
            for item in [
                &Item::ARROW,
                &Item::SNOWBALL,
                &Item::EGG,
                &Item::SPLASH_POTION,
                &Item::LINGERING_POTION,
                &Item::EXPERIENCE_BOTTLE,
                &Item::FIRE_CHARGE,
                &Item::WIND_CHARGE,
                &Item::FIREWORK_ROCKET,
            ] {
                dispenser.set_stack(0, ItemStack::new(1, item));
                let before = world.entities.load().len();
                DispenserBlock.on_scheduled_tick(OnScheduledTickArgs {
                    world: &world,
                    block: &Block::DISPENSER,
                    position: &position,
                });
                assert_eq!(
                    world.entities.load().len() - before,
                    usize::from(veto.is_none()),
                    "{} admission",
                    item.registry_key
                );
                assert_eq!(
                    dispenser.get_stack(0).item_count,
                    u8::from(veto.is_some()),
                    "{} payment",
                    item.registry_key
                );
            }
            if veto.is_some() {
                assert_eq!(calls.load(SeqCst), 9);
            }
        })
        .await;
    }
}
