#![expect(
    clippy::unwrap_used,
    clippy::unreachable,
    reason = "Decode emitted item publication packets"
)]

use pumpkin_data::{
    data_component_impl::CustomNameImpl, entity::EntityType, item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{Nbt, fixed_le::NbtReadHelperFixedLe, tag::NbtTag};
use pumpkin_protocol::{
    bedrock::{
        client::{
            CAddItemActor, inventory_content::CInventoryContent, inventory_slot::CInventorySlot,
        },
        network_item::{ContainerName, FullContainerName, NetworkItemStackDescriptor},
    },
    codec::{var_int::VarInt, var_long::VarLong, var_uint::VarUInt, var_ulong::VarULong},
    packet::Packet,
    serial::PacketRead,
};
use pumpkin_util::math::vector3::Vector3;

use super::{
    inventory_exploit_tests::packet_payload,
    loom_workstation_tests::{banner, support::LoomFixture},
};
use crate::entity::{Entity, EntityBase, item::ItemEntity};

fn item_nbt(bytes: &[u8]) -> Nbt {
    let mut cursor = std::io::Cursor::new(bytes);
    assert_eq!(i16::read(&mut cursor).unwrap(), -1);
    assert_eq!(u8::read(&mut cursor).unwrap(), 1);
    // Protocol-derived named-root prefix: Compound + fixed uint16 empty name.
    assert_eq!(&bytes[3..6], &[10, 0, 0]);
    let nbt = Nbt::read(&mut NbtReadHelperFixedLe::new(&mut cursor)).unwrap();
    assert_eq!(i32::read(&mut cursor).unwrap(), 0);
    assert_eq!(i32::read(&mut cursor).unwrap(), 0);
    assert_eq!(cursor.position() as usize, bytes.len());
    nbt
}

fn assert_components(bytes: &[u8], case: usize) {
    let nbt = item_nbt(bytes);
    match case {
        0 => {
            assert!(nbt.get_list("Patterns").unwrap().is_empty());
            assert!(nbt.get_compound("display").is_none());
            assert_eq!(
                bytes,
                &[
                    0xff, 0xff, 1, 10, 0, 0, 9, 8, 0, b'P', b'a', b't', b't', b'e', b'r', b'n',
                    b's', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                ]
            );
        }
        1 => {
            assert_eq!(
                nbt.get_compound("display").unwrap().get_string("Name"),
                Some("é🎨")
            );
            assert!(nbt.get_list("Patterns").is_none());
        }
        2 => {
            assert_eq!(
                nbt.get_compound("display").unwrap().get_string("Name"),
                Some("Keep my name")
            );
            let patterns = nbt.get_list("Patterns").unwrap();
            assert_eq!(patterns.len(), 1);
            let NbtTag::Compound(layer) = &patterns[0] else {
                unreachable!()
            };
            assert_eq!(layer.get_string("Pattern"), Some("ts"));
            assert_eq!(layer.get_int("Color"), Some(4));
        }
        3 => {
            let name = nbt
                .get_compound("display")
                .unwrap()
                .get_string("Name")
                .unwrap();
            assert_eq!(name, "🎨".repeat(8191));
        }
        _ => unreachable!(),
    }
}

fn assert_inventory(packets: &[bytes::Bytes], stack: &ItemStack, case: usize) {
    let mut content_seen = false;
    let mut slot_seen = false;
    for packet in packets {
        if let Some(mut reader) = packet_payload(packet, CInventoryContent::PACKET_ID as u32) {
            let window = VarUInt::read(&mut reader).unwrap().0;
            let slots = Vec::<NetworkItemStackDescriptor>::read(&mut reader).unwrap();
            let name = FullContainerName::read(&mut reader).unwrap();
            NetworkItemStackDescriptor::read(&mut reader).unwrap();
            assert!(reader.is_empty());
            if window == 0 {
                assert_eq!(name.container_name, ContainerName::Inventory);
                assert_eq!(slots[9].net_id, Some(stack.uid));
                assert_components(&slots[9].extra_data, case);
                content_seen = true;
            }
        }
        if let Some(mut reader) = packet_payload(packet, CInventorySlot::PACKET_ID as u32) {
            let window = VarUInt::read(&mut reader).unwrap().0;
            let slot = VarUInt::read(&mut reader).unwrap().0;
            let name = Option::<FullContainerName>::read(&mut reader)
                .unwrap()
                .unwrap();
            Option::<NetworkItemStackDescriptor>::read(&mut reader).unwrap();
            let actual = NetworkItemStackDescriptor::read(&mut reader).unwrap();
            assert!(reader.is_empty());
            if window == 0 && slot == 9 && name.container_name == ContainerName::Inventory {
                assert_eq!(actual.net_id, Some(stack.uid));
                assert_components(&actual.extra_data, case);
                slot_seen = true;
            }
        }
    }
    assert!(
        content_seen && slot_seen,
        "complete inventory and slot publication"
    );
}

fn assert_drop(packet: &bytes::Bytes, entity: &ItemEntity, stack: &ItemStack, case: usize) {
    let mut reader = packet_payload(packet, CAddItemActor::PACKET_ID as u32).unwrap();
    assert_eq!(
        VarLong::read(&mut reader).unwrap().0,
        i64::from(entity.get_entity().entity_id)
    );
    assert_eq!(
        VarULong::read(&mut reader).unwrap().0,
        entity.get_entity().entity_id as u64
    );
    let expected = NetworkItemStackDescriptor::from(stack);
    assert_eq!(i16::read(&mut reader).unwrap(), expected.id);
    assert_eq!(u16::read(&mut reader).unwrap(), u16::from(stack.item_count));
    assert_eq!(VarUInt::read(&mut reader).unwrap(), expected.aux_value);
    assert!(bool::read(&mut reader).unwrap());
    assert_eq!(VarInt::read(&mut reader).unwrap().0, stack.uid.get());
    VarUInt::read(&mut reader).unwrap();
    // Read raw wrapper user-data independently of the production item reader.
    let extra = Vec::<u8>::read(&mut reader).unwrap();
    assert_components(&extra, case);
    assert_eq!(
        Vector3::<f32>::read(&mut reader).unwrap(),
        entity.bedrock_pos().to_f32_lossy()
    );
    assert_eq!(
        Vector3::<f32>::read(&mut reader).unwrap(),
        entity.get_entity().velocity.load().to_f32_lossy()
    );
    let mut metadata = entity.get_entity().bedrock_metadata().0;
    let count = VarUInt::read(&mut reader).unwrap().0;
    assert_eq!(count as usize, metadata.len());
    for _ in 0..count {
        let key = VarUInt::read(&mut reader).unwrap().0;
        let value = metadata.remove(&key).unwrap();
        assert_eq!(VarUInt::read(&mut reader).unwrap().0, value.type_id());
        assert_eq!(u8::read(&mut reader).unwrap(), value.type_id() as u8);
        let mut encoded = Vec::new();
        value.write(&mut encoded).unwrap();
        assert!(reader.starts_with(&encoded));
        reader = &reader[encoded.len()..];
    }
    assert!(!bool::read(&mut reader).unwrap(), "fishing flag");
    assert!(reader.is_empty(), "complete dropped-item packet");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r4_item_nbt_in_full_inventory_slot_and_dropped_packets() {
    let mut fixture = LoomFixture::empty().await;
    let mut named = ItemStack::new(1, &Item::DIAMOND);
    named.set_data_component(CustomNameImpl::read_data(&NbtTag::String("é🎨".into())).unwrap());
    let mut oversized = ItemStack::new(1, &Item::DIAMOND);
    oversized.set_data_component(
        CustomNameImpl::read_data(&NbtTag::String("🎨".repeat(20000).into())).unwrap(),
    );
    for (case, stack) in [
        ItemStack::new(1, &Item::WHITE_BANNER),
        named,
        banner(1, 1),
        oversized,
    ]
    .into_iter()
    .enumerate()
    {
        let original_name = stack.get_data_component::<CustomNameImpl>().cloned();
        fixture
            .bedrock
            .player
            .inventory()
            .set_stack(9, stack.clone());
        fixture.bedrock.take_packets();
        fixture
            .bedrock
            .player
            .on_screen_handler_opened(&fixture.bedrock.player.player_screen_handler);
        fixture.bedrock.player.sync_hand_slot(9, stack.clone());
        assert_inventory(&fixture.bedrock.take_packets(), &stack, case);
        let entity = ItemEntity::new(
            Entity::new(
                fixture.world.world(),
                Vector3::new(8.5, 64.0, 8.5),
                &EntityType::ITEM,
            ),
            stack.clone(),
        );
        entity.send_bedrock_spawn_packet(fixture.bedrock.client());
        let packets = fixture.bedrock.take_packets();
        assert_eq!(packets.len(), 1);
        assert_drop(&packets[0], &entity, &stack, case);
        assert_eq!(
            stack.get_data_component::<CustomNameImpl>(),
            original_name.as_ref()
        );
    }
    fixture.close().await;
}
