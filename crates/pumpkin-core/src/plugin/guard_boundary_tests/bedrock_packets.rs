use super::*;
use crate::net::bedrock::combat_test_support::TestBedrockPlayer;
use pumpkin_protocol::{
    bedrock::{
        network_item::{ContainerName, FullContainerName},
        server::{
            inventory_transaction::{
                LegacySetItemSlot, NormalTransactionData, SInventoryTransaction, TransactionData,
            },
            item_stack_request::{
                ItemStackRequest, ItemStackRequestAction, ItemStackRequestSlotInfo,
                SItemStackRequest,
            },
        },
    },
    codec::var_uint::VarUInt,
};

struct DropReentry(Arc<AtomicUsize>);
impl EventHandler<PlayerStatisticIncrementEvent> for DropReentry {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        assert_menu_unlocked(&event.player);
        assert!(event.player.inventory.get_stack(0).is_empty());
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

async fn bedrock_drop(legacy: bool, reentry: bool) {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let player = TestBedrockPlayer::new(&world).await;
    player
        .player
        .inventory
        .set_stack(0, ItemStack::new(2, &Item::DIRT));
    let calls = Arc::new(AtomicUsize::new(0));
    if reentry {
        server
            .plugin_manager
            .register::<PlayerStatisticIncrementEvent, _>(
                Arc::new(DropReentry(calls.clone())),
                EventPriority::Normal,
                true,
            );
    }
    if legacy {
        player.client().handle_inventory_action(
            &player.player,
            SInventoryTransaction {
                legacy_request_id: VarInt(1),
                legacy_set_item_slots: vec![LegacySetItemSlot {
                    container_id: 28,
                    slots: vec![0],
                }],
                has_value: false,
                actions: vec![],
                transaction_type: VarUInt(0),
                transaction_data: TransactionData::Normal(NormalTransactionData),
            },
        );
    } else {
        player.client().handle_item_stack_request(
            &player.player,
            SItemStackRequest {
                requests: vec![ItemStackRequest {
                    request_id: VarInt(1),
                    actions: vec![ItemStackRequestAction::Drop {
                        count: 2,
                        source: ItemStackRequestSlotInfo {
                            container_name: FullContainerName {
                                container_name: ContainerName::HotBar,
                                dynamic_id: None,
                            },
                            slot_id: 0,
                            stack_id: 0,
                        },
                        randomly: false,
                    }],
                    filter_strings: vec![],
                    filter_cause: 0,
                }],
            },
        );
    }
    assert!(player.player.inventory.get_stack(0).is_empty());
    assert_eq!(world.entities.load().len(), 1);
    if reentry {
        assert!(calls.load(Ordering::Relaxed) > 0);
    }
    player.close().await;
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_bedrock_stack_request_drop_without_plugins() {
    bedrock_drop(false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_bedrock_legacy_drop_without_plugins() {
    bedrock_drop(true, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_bedrock_stack_request_drop_callback_reentry() {
    bedrock_drop(false, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_bedrock_legacy_drop_callback_reentry() {
    bedrock_drop(true, true).await;
}
