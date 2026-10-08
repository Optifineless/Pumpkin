use crate::{
    ServerPacket, codec::data_component::deserialize, java::server::play::SSetCreativeSlot,
};
use pumpkin_data::data_component::DataComponent;
use pumpkin_util::version::JavaMinecraftVersion;

#[test]
fn nested_template_uses_plain_component_patch() {
    // UseRemainder: item 1, count 1, add 1, remove 0, max_damage(id 2) = 7.
    let mut input: &[u8] = &[1, 1, 1, 0, 2, 7];
    let decoded = deserialize(DataComponent::UseRemainder, &mut input).unwrap();
    assert!(input.is_empty());
    let remainder = pumpkin_data::data_component_impl::get::<
        pumpkin_data::data_component_impl::UseRemainderImpl,
    >(&*decoded)
    .create()
    .unwrap();
    assert_eq!(remainder.item, &pumpkin_data::item::Item::STONE);
    assert_eq!(remainder.item_count, 1);
    assert_eq!(
        remainder
            .get_data_component::<pumpkin_data::data_component_impl::MaxDamageImpl>()
            .unwrap()
            .max_damage,
        7
    );
    let mut encoded = Vec::new();
    crate::codec::data_component::serialize(DataComponent::UseRemainder, &*decoded, &mut encoded)
        .unwrap();
    assert_eq!(encoded, [1, 1, 1, 0, 2, 7]);
}

#[test]
fn survival_creative_slot_decoder_bounds_recursive_remainders() {
    // Delimited outer UseRemainder; each inner template uses a plain patch.
    let mut payload = Vec::new();
    for _ in 0..1000 {
        payload.extend_from_slice(&[1, 1, 1, 0, 25]);
    }
    payload.extend_from_slice(&[1, 1, 0, 0]);
    let mut packet = vec![0, 0, 1, 1, 1, 0, 25];
    packet.extend_from_slice(&[0x8c, 0x27]); // VarInt 5004.
    packet.extend_from_slice(&payload);
    assert!(matches!(
        SSetCreativeSlot::read_bounded(&mut packet.as_slice(), &JavaMinecraftVersion::V_26_3),
        Err(crate::ser::ReadingError::TooLarge(_))
    ));
}

#[test]
fn component_collections_reject_vanilla_bounds_before_elements() {
    for (id, bytes) in [
        (DataComponent::ChargedProjectiles, &[0x81, 0x08][..]), // 1025
        (DataComponent::Container, &[0x81, 0x02][..]),          // 257
        (DataComponent::WritableBookContent, &[101][..]),
    ] {
        assert!(matches!(
            deserialize(id, &mut &bytes[..]),
            Err(crate::ser::ReadingError::TooLarge(_))
        ));
    }
}

#[test]
fn component_collections_accept_the_vanilla_edges() {
    let mut projectiles = vec![0x80, 0x08]; // 1024 templates, item 1/count 1/no patch.
    for _ in 0..1024 {
        projectiles.extend_from_slice(&[1, 1, 0, 0]);
    }
    assert!(
        deserialize(
            DataComponent::ChargedProjectiles,
            &mut projectiles.as_slice()
        )
        .is_ok()
    );
    let mut container = vec![0x80, 0x02]; // 256 empty optional slots.
    container.resize(258, 0);
    assert!(deserialize(DataComponent::Container, &mut container.as_slice()).is_ok());
    let mut pages = vec![100]; // 100 empty, unfiltered writable pages.
    pages.resize(201, 0);
    assert!(deserialize(DataComponent::WritableBookContent, &mut pages.as_slice()).is_ok());
}

#[test]
fn writable_pages_and_written_titles_use_vanilla_string_limits() {
    assert!(matches!(
        deserialize(
            DataComponent::WritableBookContent,
            &mut &[1, 0x81, 0x18][..]
        ),
        Err(crate::ser::ReadingError::TooLarge(_))
    ));
    assert!(matches!(
        deserialize(DataComponent::WrittenBookContent, &mut &[97][..]),
        Err(crate::ser::ReadingError::TooLarge(_))
    ));
}

#[test]
fn packet_work_budget_is_shared_by_sibling_components() {
    let _scope = crate::ser::decode_budget::DecodeScope::packet();
    // Empty optional container slots still consume decode work: 256 each.
    let mut bytes = vec![0x80, 0x02];
    bytes.resize(258, 0);
    for _ in 0..255 {
        assert!(deserialize(DataComponent::Container, &mut bytes.as_slice()).is_ok());
    }
    assert!(matches!(
        deserialize(DataComponent::Container, &mut bytes.as_slice()),
        Err(crate::ser::ReadingError::TooLarge(_))
    ));
}

#[test]
fn skipped_component_lists_still_consume_packet_work() {
    // DeathProtection's unbounded consume-effect list, VarInt 65537, no element bytes.
    assert!(matches!(
        deserialize(DataComponent::DeathProtection, &mut &[0x81, 0x80, 0x04][..]),
        Err(crate::ser::ReadingError::TooLarge(_))
    ));
}
