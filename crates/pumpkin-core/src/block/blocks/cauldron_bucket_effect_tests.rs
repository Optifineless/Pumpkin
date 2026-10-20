use super::{
    fillings,
    support::{Fixture, POS},
};
use crate::{
    entity::player::Player,
    plugin::{
        BoxFuture, EventHandler, EventPriority, entity::item_spawn::ItemSpawnEvent,
        player::player_statistic_increment::PlayerStatisticIncrementEvent,
    },
    server::Server,
};
use pumpkin_data::{
    Block,
    item::Item,
    item_stack::ItemStack,
    statistic::{CustomStatistic, StatisticCategory},
};
use pumpkin_inventory::Inventory;
use pumpkin_util::Hand;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct ReplaceSource {
    player: Arc<Player>,
    slot: usize,
    ran: AtomicBool,
}

impl ReplaceSource {
    fn replace(&self) {
        self.ran.store(true, Ordering::Relaxed);
        self.player
            .inventory()
            .set_stack(self.slot, ItemStack::new(7, &Item::DIAMOND));
        self.player.inventory().set_selected_slot(1);
    }
}

impl EventHandler<ItemSpawnEvent> for ReplaceSource {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut ItemSpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            assert_eq!(event.item_name, "bucket");
            self.replace();
        })
    }
}

impl EventHandler<PlayerStatisticIncrementEvent> for ReplaceSource {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.statistic_id
                == format!(
                    "{:?}:{}",
                    StatisticCategory::Custom,
                    CustomStatistic::FillCauldron as i32
                )
            {
                self.replace();
            }
        })
    }
}

async fn late_replacement(full: bool) {
    for hand in Hand::all() {
        let mut f = Fixture::new();
        let slot = if hand == Hand::Right { 0 } else { 40 };
        let handler = Arc::new(ReplaceSource {
            player: f.client.player.clone(),
            slot,
            ran: false.into(),
        });
        if full {
            f.server.plugin_manager.register::<ItemSpawnEvent, _>(
                handler.clone(),
                EventPriority::Normal,
                true,
            );
        } else {
            f.server
                .plugin_manager
                .register::<PlayerStatisticIncrementEvent, _>(
                    handler.clone(),
                    EventPriority::Normal,
                    true,
                );
        }
        for (item, target) in fillings() {
            f.reset(
                Block::CAULDRON.default_state.id,
                hand,
                ItemStack::new(2, item),
            );
            if full {
                for index in 0..36 {
                    f.client
                        .player
                        .inventory()
                        .set_stack(index, ItemStack::new(64, &Item::STONE));
                }
                f.client
                    .player
                    .inventory()
                    .set_stack(slot, ItemStack::new(2, item));
            }
            handler.ran.store(false, Ordering::Relaxed);
            f.use_top(hand);
            assert!(handler.ran.load(Ordering::Relaxed), "real callback ran");
            let current = f.client.player.inventory().get_stack(slot);
            assert_eq!(current.item, &Item::DIAMOND);
            assert_eq!(
                current.item_count, 7,
                "late source replacement survives Java writeback"
            );
            assert_eq!(f.client.player.inventory().get_selected_slot(), 1);
            assert_eq!(f.world.get_block_state_id(&POS), target);
            assert_eq!(f.count(item), 0, "callback replaced the paid remainder");
            assert_eq!(f.count(&Item::BUCKET), u32::from(!full));
            let drops = f.drops();
            assert_eq!(drops.len(), usize::from(full));
            if full {
                assert_eq!(drops[0].item, &Item::BUCKET);
                assert_eq!(drops[0].item_count, 1);
            }
        }
        f.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_stat_callback_preserves_replacement_after_payment() {
    late_replacement(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_overflow_callback_preserves_replacement_after_payment() {
    late_replacement(true).await;
}
