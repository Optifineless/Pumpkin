use super::created_output_tests::{set_logs, slot};
use crate::{
    entity::death_test_world::DeathTestWorld, net::bedrock::combat_test_support::TestBedrockPlayer,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    bedrock::{
        network_item::ContainerName,
        server::item_stack_request::{
            ItemStackRequest, ItemStackRequestAction, ItemStackRequestSlotInfo, SItemStackRequest,
        },
    },
    codec::{var_int::VarInt, var_uint::VarUInt},
};

// Logs and planks are measured in recipe-output units; the preview is not owned stock.
fn conserved_stock(
    bedrock: &TestBedrockPlayer,
    virtual_stock: &[ItemStack],
    unclaimed: &[ItemStack],
    planks_per_log: u32,
) -> u32 {
    let value = |stack: ItemStack| {
        if stack.is_empty() {
            0
        } else if stack.item == &Item::OAK_LOG {
            u32::from(stack.item_count) * planks_per_log
        } else {
            u32::from(stack.item_count)
        }
    };
    let inventory: u32 = (0..bedrock.player.inventory.size())
        .map(|slot| value(bedrock.player.inventory.get_stack(slot)))
        .sum();
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    let handler = screen.lock().unwrap();
    let grid: u32 = handler.get_behaviour().slots[1..5]
        .iter()
        .map(|slot| value(slot.get_cloned_stack()))
        .sum();
    inventory
        + grid
        + value(handler.get_behaviour().cursor_stack.lock().unwrap().clone())
        + virtual_stock
            .iter()
            .chain(unclaimed)
            .cloned()
            .map(value)
            .sum::<u32>()
}

fn generated_batch(repetitions: u8) -> ItemStackRequestAction {
    ItemStackRequestAction::CraftRecipe {
        recipe_id: VarUInt(1),
        repetitions,
    }
}

fn take_output(count: u8, destination: ItemStackRequestSlotInfo) -> ItemStackRequestAction {
    ItemStackRequestAction::Take {
        count,
        source: slot(ContainerName::CreatedOutput, 0),
        destination,
    }
}

pub(super) fn assert_response(bedrock: &mut TestBedrockPlayer, expected: u8) {
    use pumpkin_protocol::packet::Packet;
    use pumpkin_protocol::{
        bedrock::client::item_stack_response::CItemStackResponse, serial::PacketRead,
    };
    let responses = bedrock
        .take_packets()
        .into_iter()
        .filter_map(|packet| {
            assert_eq!(packet[0], pumpkin_protocol::bedrock::BEDROCK_GAME_PACKET);
            let mut reader = std::io::Cursor::new(&packet[1..]);
            let _length = VarUInt::read(&mut reader).unwrap();
            let header = VarUInt::read(&mut reader).ok()?;
            if header.0 != CItemStackResponse::PACKET_ID as u32 {
                return None;
            }
            assert_eq!(VarUInt::read(&mut reader).unwrap().0, 1);
            Some(u8::read(&mut reader).unwrap())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        responses,
        vec![expected],
        "handler must report the action result"
    );
}

async fn conservation_request(
    logs: u8,
    occupied: u8,
    actions: Vec<ItemStackRequestAction>,
) -> (DeathTestWorld, TestBedrockPlayer) {
    let fixture = DeathTestWorld::new().await;
    let mut bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    set_logs(&bedrock, logs);
    if occupied > 0 {
        bedrock
            .player
            .inventory
            .set_stack(10, ItemStack::new(occupied, &Item::OAK_PLANKS));
    }
    run_conserved_request(&mut bedrock, actions, 1);
    (fixture, bedrock)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_second_recipe_does_not_inherit_payment() {
    let (fixture, bedrock) = conservation_request(
        2,
        0,
        vec![
            generated_batch(1),
            take_output(4, slot(ContainerName::Inventory, 9)),
            generated_batch(1),
        ],
    )
    .await;
    assert_eq!(bedrock.player.inventory.get_stack(9).item_count, 4);
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    assert_eq!(
        screen.lock().unwrap().get_behaviour().slots[1]
            .get_cloned_stack()
            .item_count,
        1
    );
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_unmapped_destination_retains_paid_stock() {
    let (fixture, bedrock) = conservation_request(
        1,
        0,
        vec![
            generated_batch(1),
            take_output(1, slot(ContainerName::Inventory, 9)),
            take_output(3, slot(ContainerName::Inventory, 36)),
        ],
    )
    .await;
    let total: u32 = (0..bedrock.player.inventory.size())
        .map(|slot| bedrock.player.inventory.get_stack(slot))
        .filter(|stack| stack.item == &Item::OAK_PLANKS)
        .map(|stack| u32::from(stack.item_count))
        .sum();
    assert_eq!(total, 0, "preflight must reject before the first payment");
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    assert_eq!(
        screen.lock().unwrap().get_behaviour().slots[1]
            .get_cloned_stack()
            .item_count,
        1
    );
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_full_destination_retains_all_251_paid_planks() {
    let (fixture, bedrock) = conservation_request(
        63,
        64,
        vec![
            generated_batch(63),
            take_output(1, slot(ContainerName::Inventory, 9)),
            take_output(251, slot(ContainerName::Inventory, 10)),
        ],
    )
    .await;
    assert_eq!(bedrock.player.inventory.get_stack(10).item_count, 64);
    let total: u32 = (0..bedrock.player.inventory.size())
        .map(|slot| bedrock.player.inventory.get_stack(slot))
        .filter(|stack| stack.item == &Item::OAK_PLANKS)
        .map(|stack| u32::from(stack.item_count))
        .sum();
    assert_eq!(total, 316);
    bedrock.close().await;
    fixture.server.shutdown().await;
}

// At handler return both request-local stores are empty: unpaid previews were discarded
// and paid leftovers were returned. Sampling here includes every externally owned item.
fn run_conserved_request(
    bedrock: &mut TestBedrockPlayer,
    actions: Vec<ItemStackRequestAction>,
    expected_result: u8,
) {
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    let ratio = u32::from(
        screen.lock().unwrap().get_behaviour().slots[0]
            .get_cloned_stack()
            .item_count,
    );
    assert!(ratio > 0);
    let before = conserved_stock(bedrock, &[], &[], ratio);
    bedrock.take_packets();
    bedrock.client().handle_item_stack_request(
        &bedrock.player,
        SItemStackRequest {
            requests: vec![ItemStackRequest {
                request_id: VarInt(-1),
                actions,
                filter_strings: Vec::new(),
                filter_cause: 0,
            }],
        },
    );
    // Virtual and unclaimed stock are request-local and must be settled before this entry point returns.
    assert_eq!(
        conserved_stock(bedrock, &[], &[], ratio),
        before,
        "request created or destroyed stock"
    );
    assert_response(bedrock, expected_result);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_incompatible_destination_does_not_pay_pending_batch() {
    let fixture = DeathTestWorld::new().await;
    let mut bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    set_logs(&bedrock, 2);
    let sentinel = ItemStack::new(7, &Item::DIAMOND);
    bedrock.player.inventory.set_stack(10, sentinel.clone());
    run_conserved_request(
        &mut bedrock,
        vec![
            generated_batch(1),
            take_output(4, slot(ContainerName::Inventory, 10)),
        ],
        1,
    );
    assert!(bedrock.player.inventory.get_stack(9).is_empty());
    assert!(bedrock.player.inventory.get_stack(10).are_equal(&sentinel));
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_incompatible_destination_returns_paid_remainder() {
    let fixture = DeathTestWorld::new().await;
    let mut bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    set_logs(&bedrock, 2);
    let sentinel = ItemStack::new(7, &Item::DIAMOND);
    bedrock.player.inventory.set_stack(10, sentinel.clone());
    run_conserved_request(
        &mut bedrock,
        vec![
            generated_batch(1),
            take_output(1, slot(ContainerName::Inventory, 9)),
            take_output(3, slot(ContainerName::Inventory, 10)),
        ],
        1,
    );
    assert_eq!(bedrock.player.inventory.get_stack(9).item_count, 4);
    let unchanged = bedrock.player.inventory.get_stack(10);
    assert_eq!(unchanged.uid, sentinel.uid);
    assert!(unchanged.are_equal(&sentinel));
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_rejects_batch_count_overflow() {
    let fixture = DeathTestWorld::new().await;
    let mut bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    // 64 logs make 256 planks: reject the batch rather than saturate its request-local u8 count.
    set_logs(&bedrock, 64);
    run_conserved_request(
        &mut bedrock,
        vec![
            generated_batch(64),
            take_output(1, slot(ContainerName::Inventory, 9)),
        ],
        1,
    );
    assert!(bedrock.player.inventory.get_stack(9).is_empty());
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_partial_take_returns_remainder_and_debits_exact_insertions() {
    let fixture = DeathTestWorld::new().await;
    let mut bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    set_logs(&bedrock, 2);
    run_conserved_request(
        &mut bedrock,
        vec![
            generated_batch(1),
            take_output(1, slot(ContainerName::Inventory, 9)),
        ],
        0,
    );
    assert_eq!(bedrock.player.inventory.get_stack(9).item_count, 4);
    let existing = bedrock.player.inventory.get_stack(9);
    run_conserved_request(
        &mut bedrock,
        vec![
            generated_batch(1),
            take_output(1, slot(ContainerName::Cursor, 0)),
            ItemStackRequestAction::Place {
                count: 2,
                source: slot(ContainerName::CreatedOutput, 0),
                destination: slot(ContainerName::Inventory, 9),
            },
        ],
        0,
    );
    assert_eq!(bedrock.player.inventory.get_stack(9).item_count, 7);
    assert_eq!(bedrock.player.inventory.get_stack(9).uid, existing.uid);
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    assert_eq!(
        screen
            .lock()
            .unwrap()
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap()
            .item_count,
        1
    );
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_out_of_range_destination_retains_paid_stock() {
    let (fixture, bedrock) = conservation_request(
        1,
        0,
        vec![
            generated_batch(1),
            take_output(1, slot(ContainerName::Inventory, 9)),
            take_output(3, slot(ContainerName::HotBar, u8::MAX)),
        ],
    )
    .await;
    assert!(bedrock.player.inventory.get_stack(9).is_empty());
    let screen = bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    assert_eq!(
        screen.lock().unwrap().get_behaviour().slots[1]
            .get_cloned_stack()
            .item_count,
        1,
        "preflight must reject the entire request before the first write"
    );
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_physical_transfer_rejects_legacy_count_overflow() {
    let fixture = DeathTestWorld::new().await;
    let mut bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    set_logs(&bedrock, 1);
    let source = ItemStack::new(3, &Item::OAK_PLANKS);
    let legacy = ItemStack::new(u8::MAX, &Item::OAK_PLANKS);
    bedrock.player.inventory.set_stack(9, source.clone());
    bedrock.player.inventory.set_stack(10, legacy.clone());
    run_conserved_request(
        &mut bedrock,
        vec![ItemStackRequestAction::Place {
            count: 1,
            source: slot(ContainerName::Inventory, 9),
            destination: slot(ContainerName::Inventory, 10),
        }],
        1,
    );
    assert!(bedrock.player.inventory.get_stack(9).are_equal(&source));
    assert!(bedrock.player.inventory.get_stack(10).are_equal(&legacy));
    bedrock.close().await;
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_empty_destination_count_does_not_buy_extra_output() {
    for container in [ContainerName::Inventory, ContainerName::Cursor] {
        empty_destination_request(container, false).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_empty_destination_after_payment_cannot_amplify_stock() {
    for container in [ContainerName::Inventory, ContainerName::Cursor] {
        empty_destination_request(container, true).await;
    }
}

async fn empty_destination_request(container: ContainerName, paid: bool) {
    let fixture = DeathTestWorld::new().await;
    let mut bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    set_logs(&bedrock, 1);
    let empty = ItemStack::new(7, &Item::AIR);
    let destination = if container == ContainerName::Cursor {
        let screen = bedrock
            .player
            .current_screen_handler
            .lock()
            .unwrap()
            .clone();
        *screen
            .lock()
            .unwrap()
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap() = empty;
        slot(container, 0)
    } else {
        let source = crate::command::CommandSender::Console.into_source(&fixture.server);
        assert_eq!(
            fixture
                .server
                .command_dispatcher
                .load()
                .execute_input(
                    "item replace entity @a container.9 with minecraft:air 7",
                    &source
                )
                .unwrap(),
            1
        );
        assert!(bedrock.player.inventory.get_stack(9).is_empty());
        assert_eq!(bedrock.player.inventory.get_stack(9).item_count, 7);
        slot(container, 9)
    };
    let mut actions = vec![generated_batch(1)];
    if paid {
        actions.push(take_output(1, slot(ContainerName::Inventory, 10)));
        actions.push(take_output(3, destination));
    } else {
        actions.push(take_output(4, destination));
    }
    run_conserved_request(&mut bedrock, actions, 0);
    bedrock.close().await;
    fixture.server.shutdown().await;
}
