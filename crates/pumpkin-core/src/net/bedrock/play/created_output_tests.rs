use crate::plugin::api::events::player::player_statistic_increment::PlayerStatisticIncrementEvent;
use crate::{
    entity::death_test_world::DeathTestWorld, net::bedrock::combat_test_support::TestBedrockPlayer,
};
use crate::{
    entity::player::Player,
    plugin::{BoxFuture, EventHandler, EventPriority},
    server::Server,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::bedrock::server::item_stack_request::{
    ItemStackRequest, ItemStackRequestAction, ItemStackRequestSlotInfo, SItemStackRequest,
};
use pumpkin_protocol::{
    bedrock::network_item::{ContainerName, FullContainerName},
    codec::{var_int::VarInt, var_uint::VarUInt},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

struct CheckCraftCallback {
    player: Arc<Player>,
    calls: Arc<AtomicUsize>,
}
impl EventHandler<PlayerStatisticIncrementEvent> for CheckCraftCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        let screen = self.player.current_screen_handler.lock().unwrap().clone();
        assert!(
            screen.try_lock().is_ok(),
            "craft callback retained the menu guard"
        );
        self.calls.fetch_add(1, Relaxed);
        Box::pin(async {})
    }
}

pub(super) fn slot(name: ContainerName, slot_id: u8) -> ItemStackRequestSlotInfo {
    ItemStackRequestSlotInfo {
        container_name: FullContainerName {
            container_name: name,
            dynamic_id: None,
        },
        slot_id,
        stack_id: 0,
    }
}

fn request(craft: ItemStackRequestAction, count: u8) -> SItemStackRequest {
    SItemStackRequest {
        requests: vec![ItemStackRequest {
            request_id: VarInt(-1),
            actions: vec![
                craft,
                ItemStackRequestAction::Take {
                    count,
                    source: slot(ContainerName::CreatedOutput, 0),
                    destination: slot(ContainerName::Inventory, 9),
                },
            ],
            filter_strings: Vec::new(),
            filter_cause: 0,
        }],
    }
}

fn recipe_request(count: u8) -> SItemStackRequest {
    request(
        ItemStackRequestAction::CraftRecipe {
            recipe_id: VarUInt(1),
            repetitions: 1,
        },
        count,
    )
}

pub(super) fn set_logs(bedrock: &TestBedrockPlayer, count: u8) {
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    let handler = screen.lock().unwrap();
    handler.get_behaviour().slots[1].set_stack(ItemStack::new(count, &Item::OAK_LOG));
    handler.get_behaviour().slots[0]
        .on_quick_move_crafted(ItemStack::EMPTY.clone(), ItemStack::EMPTY.clone());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_bedrock_creative_takes_virtual_output_without_physical_source() {
    let fixture = DeathTestWorld::new().await;
    let bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    bedrock
        .player
        .gamemode
        .store(pumpkin_util::GameMode::Creative);
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    let result = screen.lock().unwrap().get_behaviour().slots[0].get_inventory();
    let sentinel = ItemStack::new(7, &Item::DIAMOND);
    result.set_stack(0, sentinel.clone());
    screen.lock().unwrap().get_behaviour_mut().tracked_stacks[0] = sentinel.clone();
    let (index, entry) = pumpkin_data::bedrock_creative::CREATIVE_ENTRIES
        .iter()
        .enumerate()
        .find(|(_, entry)| {
            pumpkin_data::item::JavaToBedrockItemMapping::from_bedrock(
                entry.item_id,
                entry.item_aux_value,
            )
            .is_some()
        })
        .unwrap();
    let mapping = pumpkin_data::item::JavaToBedrockItemMapping::from_bedrock(
        entry.item_id,
        entry.item_aux_value,
    )
    .unwrap();
    bedrock.client().handle_item_stack_request(
        &bedrock.player,
        request(
            ItemStackRequestAction::CraftCreative {
                creative_item_id: VarUInt((index + 1) as u32),
                repetitions: 1,
            },
            1,
        ),
    );
    let received = bedrock.player.inventory.get_stack(9);
    assert_eq!(
        (received.item.id, received.item_count),
        (mapping.java_item.id, 1)
    );
    assert_eq!(result.get_stack(0).uid, sentinel.uid);
    assert!(result.get_stack(0).are_equal(&sentinel));
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_bedrock_recipe_commits_virtual_output_after_validation() {
    let fixture = DeathTestWorld::new().await;
    let bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(CheckCraftCallback {
                player: bedrock.player.clone(),
                calls: calls.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    set_logs(&bedrock, 2);
    bedrock
        .client()
        .handle_item_stack_request(&bedrock.player, recipe_request(4));
    let received = bedrock.player.inventory.get_stack(9);
    assert_eq!(
        (received.item.id, received.item_count),
        (Item::OAK_PLANKS.id, 4)
    );
    assert_eq!(
        screen.lock().unwrap().get_behaviour().slots[1]
            .get_cloned_stack()
            .item_count,
        1
    );
    let refreshed = screen.lock().unwrap().get_behaviour().slots[0].get_cloned_stack();
    assert_eq!(
        (refreshed.item.id, refreshed.item_count),
        (Item::OAK_PLANKS.id, 4)
    );
    assert_eq!(calls.load(Relaxed), 1);
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_bedrock_pending_recipe_rejects_physical_preview_source() {
    let fixture = DeathTestWorld::new().await;
    let bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    set_logs(&bedrock, 2);
    let mut physical_preview = recipe_request(4);
    physical_preview.requests[0].actions[1] = ItemStackRequestAction::Take {
        count: 4,
        source: slot(ContainerName::CraftingOutputPreview, 0),
        destination: slot(ContainerName::Inventory, 9),
    };
    bedrock
        .client()
        .handle_item_stack_request(&bedrock.player, physical_preview);

    assert!(bedrock.player.inventory.get_stack(9).is_empty());
    assert_eq!(
        screen.lock().unwrap().get_behaviour().slots[1]
            .get_cloned_stack()
            .item_count,
        2
    );

    let mut already_paid = recipe_request(4);
    already_paid.requests[0]
        .actions
        .push(ItemStackRequestAction::Take {
            count: 4,
            source: slot(ContainerName::CraftingOutputPreview, 0),
            destination: slot(ContainerName::Inventory, 10),
        });
    bedrock
        .client()
        .handle_item_stack_request(&bedrock.player, already_paid);
    assert_eq!(
        bedrock.player.inventory.get_stack(9).item_count,
        0,
        "preflight must reject the entire request before paying or delivering output"
    );
    assert!(
        bedrock.player.inventory.get_stack(10).is_empty(),
        "a refilled physical preview duplicated the paid output"
    );
    assert_eq!(
        screen.lock().unwrap().get_behaviour().slots[1]
            .get_cloned_stack()
            .item_count,
        2
    );
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_bedrock_rejects_full_and_phantom_output_without_payment() {
    let fixture = DeathTestWorld::new().await;
    let bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    set_logs(&bedrock, 1);
    bedrock
        .player
        .inventory
        .set_stack(9, ItemStack::new(64, &Item::OAK_PLANKS));
    bedrock
        .client()
        .handle_item_stack_request(&bedrock.player, recipe_request(4));
    assert_eq!(bedrock.player.inventory.get_stack(9).item_count, 64);
    assert_eq!(
        screen.lock().unwrap().get_behaviour().slots[1]
            .get_cloned_stack()
            .item_count,
        1,
        "a full destination consumed recipe inputs"
    );
    bedrock
        .player
        .inventory
        .set_stack(9, ItemStack::EMPTY.clone());
    let mut phantom = recipe_request(1);
    phantom.requests[0].actions.remove(0);
    bedrock
        .client()
        .handle_item_stack_request(&bedrock.player, phantom);
    assert!(
        bedrock.player.inventory.get_stack(9).is_empty(),
        "CreatedOutput without request stock used the physical preview"
    );
    assert_eq!(
        screen.lock().unwrap().get_behaviour().slots[1]
            .get_cloned_stack()
            .item_count,
        1,
        "a CreatedOutput without request stock consumed recipe inputs"
    );
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_bedrock_returns_unclaimed_partial_recipe_output() {
    let fixture = DeathTestWorld::new().await;
    let bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    set_logs(&bedrock, 1);
    bedrock
        .client()
        .handle_item_stack_request(&bedrock.player, recipe_request(1));
    // Returning the unclaimed output may merge it into the destination.
    assert!(bedrock.player.inventory.get_stack(9).item_count >= 1);
    let total: u32 = (0..bedrock.player.inventory.size())
        .map(|slot| bedrock.player.inventory.get_stack(slot))
        .filter(|stack| stack.item == &Item::OAK_PLANKS)
        .map(|stack| u32::from(stack.item_count))
        .sum();
    assert_eq!(
        total, 4,
        "partial Take lost paid virtual output at request end"
    );
    assert!(
        screen.lock().unwrap().get_behaviour().slots[1]
            .get_cloned_stack()
            .is_empty()
    );
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_bedrock_recipe_to_cursor_and_empty_grid_cell() {
    let fixture = DeathTestWorld::new().await;
    let bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    for (container, slot_id) in [
        (ContainerName::Cursor, 0),
        (ContainerName::CraftingInput, 1),
    ] {
        set_logs(&bedrock, 1);
        let mut packet = recipe_request(4);
        if let ItemStackRequestAction::Take { destination, .. } = &mut packet.requests[0].actions[1]
        {
            *destination = slot(container, slot_id);
        }
        bedrock
            .client()
            .handle_item_stack_request(&bedrock.player, packet);
        let handler = screen.lock().unwrap();
        let received = if container == ContainerName::Cursor {
            handler.get_behaviour().cursor_stack.lock().unwrap().clone()
        } else {
            handler.get_behaviour().slots[2].get_cloned_stack()
        };
        assert_eq!(
            (received.item.id, received.item_count),
            (Item::OAK_PLANKS.id, 4)
        );
        assert!(
            handler.get_behaviour().slots[1]
                .get_cloned_stack()
                .is_empty()
        );
    }
    bedrock.close().await;
    fixture.server.shutdown().await;
}
