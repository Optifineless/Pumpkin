#![expect(
    clippy::unwrap_used,
    reason = "Decode real workstation correction packets"
)]

use bytes::Bytes;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    bedrock::{
        client::inventory_content::CInventoryContent,
        network_item::{ContainerName, FullContainerName, NetworkItemStackDescriptor},
    },
    codec::var_uint::VarUInt,
    packet::Packet,
    serial::PacketRead,
};

use super::inventory_exploit_tests::packet_payload;
use crate::entity::player::Player;

fn assert_stack(actual: &NetworkItemStackDescriptor, stack: &ItemStack) {
    let expected = NetworkItemStackDescriptor::from(stack);
    assert_eq!(actual.id, expected.id);
    assert_eq!(actual.stack_size, expected.stack_size, "corrected count");
    assert_eq!(actual.net_id, expected.net_id, "corrected stack identity");
    assert_eq!(actual.aux_value, expected.aux_value);
    assert_eq!(actual.block_runtime_id, expected.block_runtime_id);
    assert_eq!(actual.extra_data, expected.extra_data);
}

pub(super) fn assert_correction(packets: &[Bytes], player: &Player) {
    let (menu, cursor) = {
        let current = player.current_screen_handler.lock().unwrap().clone();
        let handler = current.lock().unwrap();
        let menu = handler.window_type().map(|_| {
            let slots = &handler.get_behaviour().slots;
            // PR #382 owns generic addressing. Assert its existing LevelEntity
            // menu-local vector exactly, without changing correction policy here.
            (
                u32::from(handler.sync_id()),
                ContainerName::LevelEntity,
                slots[..slots.len() - 36]
                    .iter()
                    .map(|slot| slot.get_cloned_stack())
                    .collect(),
            )
        });
        let cursor = handler.get_behaviour().cursor_stack.lock().unwrap().clone();
        (menu, cursor)
    };
    let personal = player.player_screen_handler.lock().unwrap();
    let mut expected = vec![
        (
            0,
            ContainerName::Inventory,
            (0..36)
                .map(|i| player.inventory().get_stack(i))
                .collect::<Vec<_>>(),
        ),
        (
            120,
            ContainerName::Armor,
            (5..9)
                .map(|i| personal.get_slot(i).get_cloned_stack())
                .collect(),
        ),
        (
            119,
            ContainerName::Offhand,
            vec![personal.get_slot(45).get_cloned_stack()],
        ),
    ];
    drop(personal);
    expected.extend(menu);
    expected.push((59, ContainerName::Cursor, vec![cursor.clone()]));
    let mut seen = vec![false; expected.len()];
    for packet in packets {
        if let Some(mut reader) = packet_payload(packet, CInventoryContent::PACKET_ID as u32) {
            let window = VarUInt::read(&mut reader).unwrap().0;
            let slots = Vec::<NetworkItemStackDescriptor>::read(&mut reader).unwrap();
            let name = FullContainerName::read(&mut reader).unwrap();
            let storage = NetworkItemStackDescriptor::read(&mut reader).unwrap();
            assert_eq!(name.dynamic_id, None);
            assert_stack(&storage, ItemStack::EMPTY);
            assert!(reader.is_empty());
            let index = expected
                .iter()
                .position(|(id, container, _)| *id == window && *container == name.container_name)
                .unwrap();
            let stock = &expected[index].2;
            assert_eq!(slots.len(), stock.len(), "correction slot vector length");
            for (actual, stack) in slots.iter().zip(stock) {
                assert_stack(actual, stack);
            }
            seen[index] = true;
        }
    }
    assert!(
        seen[..seen.len() - 1].iter().all(|seen| *seen),
        "missing exact inventory/menu correction: {seen:?}"
    );
    if !cursor.is_empty() {
        assert!(
            seen[seen.len() - 1],
            "dirty cursor must publish its count and identity"
        );
    }
}
