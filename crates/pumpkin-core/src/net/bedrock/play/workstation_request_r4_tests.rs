#![expect(
    clippy::unwrap_used,
    clippy::unreachable,
    reason = "Real workstation request regressions"
)]

use pumpkin_data::{item_stack::ItemStack, screen::WindowType};
use pumpkin_inventory::{Inventory, beacon_screen_handler::BeaconScreenHandler};
use pumpkin_protocol::{
    bedrock::{
        client::{inventory_slot::CInventorySlot, item_stack_response::CItemStackResponse},
        network_item::{ContainerName, FullContainerName, NetworkItemStackDescriptor},
        server::item_stack_request::{
            ItemStackRequest, ItemStackRequestAction as Action,
            ItemStackRequestSlotInfo as RequestSlot, SItemStackRequest,
        },
    },
    codec::{var_int::VarInt, var_uint::VarUInt},
    java::server::play::SSetBeacon,
    packet::Packet,
    serial::PacketRead,
};

use super::{
    inventory_exploit_tests::packet_payload,
    inventory_request_beacon_tests::support::Fixture,
    loom_workstation_tests::{
        banner,
        support::{LoomFixture, assert_same_stock, responses, slot},
    },
};
use crate::{block::entities::BlockEntity, net::bedrock::combat_test_support::TestBedrockPlayer};

fn send(owner: &TestBedrockPlayer, id: i32, actions: Vec<Action>) {
    owner.client().handle_item_stack_request(
        &owner.player,
        SItemStackRequest {
            requests: vec![ItemStackRequest {
                request_id: VarInt(id),
                actions,
                filter_strings: Vec::new(),
                filter_cause: 0,
            }],
        },
    );
}

fn output_action(actions: &mut [Action]) -> (&mut RequestSlot, &mut RequestSlot) {
    let Action::Take {
        source,
        destination,
        ..
    } = &mut actions[4]
    else {
        unreachable!()
    };
    (source, destination)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r4_preview_predictions_follow_live_output() {
    for alias in [
        ContainerName::LoomResultPreview,
        ContainerName::CraftingOutputPreview,
    ] {
        let mut fixture = LoomFixture::new(banner(3, 1), 3, None).await;
        let index = {
            let mut handler = fixture.screen.lock().unwrap();
            let loom = handler
                .as_any_mut()
                .downcast_mut::<pumpkin_inventory::loom_screen_handler::LoomScreenHandler>()
                .unwrap();
            loom.slots_changed();
            loom.selectable_patterns
                .iter()
                .position(|pattern| pattern == "minecraft:stripe_bottom")
                .unwrap()
        };
        assert!(
            fixture
                .screen
                .lock()
                .unwrap()
                .on_button_click(fixture.bedrock.player.as_ref(), index as i32)
        );
        let pattern = "bs";
        let mut first = fixture.craft_actions(pattern, 1, false);
        *output_action(&mut first).0 = slot(alias, 50, &fixture.preview());
        fixture.bedrock.take_packets();
        send(&fixture.bedrock, -10, first);
        let mut second = fixture.craft_actions(pattern, 1, false);
        let (source, destination) = output_action(&mut second);
        *source = slot(alias, 50, &fixture.preview());
        source.stack_id = -10;
        *destination = slot(ContainerName::Inventory, 13, ItemStack::EMPTY);
        send(&fixture.bedrock, -11, second);
        let decoded = responses(&fixture.bedrock.take_packets());
        assert_eq!(
            decoded
                .iter()
                .map(|(id, status, _)| (*id, *status))
                .collect::<Vec<_>>(),
            [(-10, 0), (-11, 0)]
        );
        assert_eq!(fixture.input(0).item_count, 1);
        assert_eq!(fixture.input(1).item_count, 1);
        assert_eq!(
            fixture.bedrock.player.inventory().get_stack(13).item_count,
            1
        );
        fixture.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r4_destination_predictions_pay_each_craft() {
    for (name, index) in [
        (ContainerName::Inventory, 12),
        (ContainerName::CombinedHotBarAndInventory, 12),
        (ContainerName::HotBar, 3),
        (ContainerName::Cursor, 0),
    ] {
        let mut fixture = LoomFixture::new(banner(3, 1), 3, None).await;
        fixture.bedrock.take_packets();
        for (id, previous) in [(-10, 0), (-11, -10)] {
            let mut actions = fixture.craft_actions("bs", 1, name == ContainerName::Cursor);
            let (source, destination) = output_action(&mut actions);
            source.stack_id = id;
            *destination = slot(name, index, ItemStack::EMPTY);
            destination.stack_id = previous;
            send(&fixture.bedrock, id, actions);
        }
        let decoded = responses(&fixture.bedrock.take_packets());
        assert_eq!(
            decoded
                .iter()
                .map(|(id, status, _)| (*id, *status))
                .collect::<Vec<_>>(),
            [(-10, 0), (-11, 0)]
        );
        let destination = if name == ContainerName::Cursor {
            fixture.cursor()
        } else {
            fixture
                .bedrock
                .player
                .inventory()
                .get_stack(usize::from(index))
        };
        assert_eq!(destination.item_count, 2);
        assert_eq!(
            (fixture.input(0).item_count, fixture.input(1).item_count),
            (1, 1)
        );
        fixture.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r4_predictions_reject_replaced_uid_and_wrong_address() {
    for case in [
        "destination_uid",
        "destination_address",
        "preview_uid",
        "created_output",
    ] {
        let mut fixture = LoomFixture::new(banner(3, 1), 3, None).await;
        let actions = fixture.craft_actions("bs", 1, false);
        fixture.submit(-10, actions, 0);
        let mut actions = fixture.craft_actions("bs", 1, false);
        let (source, destination) = output_action(&mut actions);
        source.stack_id = -11;
        match case {
            "destination_uid" => {
                let replacement = fixture
                    .bedrock
                    .player
                    .inventory()
                    .get_stack(12)
                    .copy_with_count(1);
                fixture
                    .bedrock
                    .player
                    .inventory()
                    .set_stack(12, replacement);
                destination.stack_id = -10;
            }
            "destination_address" => {
                destination.container_name.container_name =
                    ContainerName::CombinedHotBarAndInventory;
                destination.stack_id = -10;
            }
            "preview_uid" => {
                let mut screen = fixture.screen.lock().unwrap();
                let loom = screen
                    .as_any_mut()
                    .downcast_mut::<pumpkin_inventory::loom_screen_handler::LoomScreenHandler>()
                    .unwrap();
                loom.output_inventory
                    .set_stack(0, loom.output_inventory.get_stack(0).copy_with_count(1));
                *source = slot(ContainerName::LoomResultPreview, 50, ItemStack::EMPTY);
                source.stack_id = -10;
            }
            "created_output" => source.stack_id = -10,
            _ => unreachable!(),
        }
        let before = fixture.snapshot();
        fixture.submit(-11, actions, 1);
        assert_same_stock(&before, &fixture.snapshot(), "rejected r4 request");
        fixture.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r4_invalid_cursor_index_conserves_stock() {
    let mut fixture = LoomFixture::new(banner(2, 1), 2, None).await;
    let mut actions = fixture.craft_actions("bs", 1, true);
    output_action(&mut actions).1.slot_id = 255;
    let before = fixture.snapshot();
    let selection = fixture.selection();
    fixture.submit(-10, actions, 1);
    assert_same_stock(&before, &fixture.snapshot(), "rejected r4 request");
    assert_eq!(fixture.selection(), selection);
    let actions = fixture.craft_actions("bs", 1, true);
    fixture.submit(
        -11,
        {
            let mut actions = actions;
            output_action(&mut actions).0.stack_id = -11;
            actions
        },
        0,
    );
    assert_eq!(fixture.cursor().item_count, 1);
    fixture.close().await;
}

fn assert_pending(fixture: &Fixture) {
    let pos = fixture.beacon.get_position();
    assert!(
        fixture
            .world
            .level
            .loaded_chunks
            .get(&pos.chunk_position())
            .unwrap()
            .pending_block_entities
            .lock()
            .unwrap()
            .contains_key(&pos)
    );
}

async fn pending_beacon(native_validity: bool) {
    let mut fixture = Fixture::new(1).await;
    fixture.pay();
    assert_pending(&fixture);
    let payment = fixture.payment().get_stack(0);
    let effects = fixture.effects();
    let pos = fixture.beacon.get_position();
    if native_validity {
        *fixture
            .observer
            .player
            .current_screen_handler
            .lock()
            .unwrap() = fixture.screen();
        fixture
            .world
            .block_entities
            .get_mut(&pos.chunk_position())
            .unwrap()
            .remove(&pos);
        fixture.observer.client().handle_set_beacon(
            &fixture.observer.player,
            &SSetBeacon {
                primary_effect: Some(VarInt(0)),
                secondary_effect: None,
            },
        );
    } else {
        let world = fixture.world.clone();
        fixture
            .screen()
            .lock()
            .unwrap()
            .as_any_mut()
            .downcast_mut::<BeaconScreenHandler>()
            .unwrap()
            .validity_check = Some(Box::new(move |_| {
            world
                .block_entities
                .get_mut(&pos.chunk_position())
                .unwrap()
                .remove(&pos);
            true
        }));
        let actions = fixture.selection_actions(1, 0);
        fixture.owner.take_packets();
        send(&fixture.owner, -10, actions);
        assert_eq!(responses(&fixture.owner.take_packets())[0].1, 1);
    }
    assert_eq!(fixture.payment().get_stack(0).uid, payment.uid);
    assert!(fixture.payment().get_stack(0).are_equal(&payment));
    assert_eq!(fixture.effects(), effects);
    assert_pending(&fixture);
    assert!(
        !fixture
            .world
            .block_entities
            .get(&pos.chunk_position())
            .unwrap()
            .contains_key(&pos)
    );
    fixture.world.add_block_entity(fixture.beacon.clone());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r4_pending_beacon_bedrock_never_promotes_under_menu_guards() {
    pending_beacon(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r4_pending_beacon_native_validity_never_promotes_under_menu_guards() {
    pending_beacon(true).await;
}

fn assert_post_response_slots(
    packets: &[bytes::Bytes],
    window: u32,
    expected: &[(ContainerName, u8, ItemStack)],
) {
    let response = packets
        .iter()
        .position(|packet| packet_payload(packet, CItemStackResponse::PACKET_ID as u32).is_some())
        .unwrap();
    let mut found = vec![false; expected.len()];
    for (position, packet) in packets.iter().enumerate() {
        let Some(mut reader) = packet_payload(packet, CInventorySlot::PACKET_ID as u32) else {
            continue;
        };
        let actual_window = VarUInt::read(&mut reader).unwrap().0;
        let slot = VarUInt::read(&mut reader).unwrap().0;
        let name = Option::<FullContainerName>::read(&mut reader)
            .unwrap()
            .unwrap();
        Option::<NetworkItemStackDescriptor>::read(&mut reader).unwrap();
        let stack = NetworkItemStackDescriptor::read(&mut reader).unwrap();
        assert!(reader.is_empty());
        assert!(
            position > response,
            "workstation slot publication follows acknowledgment"
        );
        assert_eq!(actual_window, window);
        assert_ne!(
            name.container_name,
            ContainerName::LevelEntity,
            "workstation UI address"
        );
        for (index, (expected_name, expected_slot, expected_stack)) in expected.iter().enumerate() {
            if name.container_name == *expected_name && slot == u32::from(*expected_slot) {
                assert_eq!(stack.stack_size, u16::from(expected_stack.item_count));
                assert_eq!(
                    stack.net_id,
                    (!expected_stack.is_empty()).then_some(expected_stack.uid)
                );
                found[index] = true;
            }
        }
    }
    assert!(
        found.iter().all(|found| *found),
        "all changed physical slots published: {found:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r4_post_response_slots_use_workstation_addresses() {
    let mut loom = LoomFixture::new(banner(3, 1), 3, None).await;
    let actions = loom.craft_actions("bs", 1, false);
    loom.submit(-10, actions, 0);
    let window = u32::from(loom.screen.lock().unwrap().sync_id());
    assert_eq!(
        loom.screen.lock().unwrap().window_type(),
        Some(WindowType::Loom)
    );
    assert_post_response_slots(
        &loom.last_packets,
        window,
        &[
            (ContainerName::LoomInput, 9, loom.input(0)),
            (ContainerName::LoomDye, 10, loom.input(1)),
            (ContainerName::LoomResultPreview, 50, loom.preview()),
            (
                ContainerName::Inventory,
                12,
                loom.bedrock.player.inventory().get_stack(12),
            ),
        ],
    );
    loom.close().await;
    let mut beacon = Fixture::new(1).await;
    beacon.pay();
    let actions = beacon.selection_actions(1, 0);
    beacon.owner.take_packets();
    send(&beacon.owner, -10, actions);
    let window = u32::from(beacon.screen().lock().unwrap().sync_id());
    assert_post_response_slots(
        &beacon.owner.take_packets(),
        window,
        &[(ContainerName::BeaconPayment, 27, ItemStack::EMPTY.clone())],
    );
    beacon.finish().await;
}
