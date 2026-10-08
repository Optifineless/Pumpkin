use pumpkin_data::{data_component::DataComponent, item::Item};
use pumpkin_util::version::JavaMinecraftVersion;

use super::{data_component::DataComponentCodec, item_stack_seralizer::ItemStackSerializer};
use crate::{ServerPacket, VarInt, ser::NetworkWriteExt};

#[expect(
    clippy::unwrap_used,
    reason = "Independent wire fixtures must decode or fail their assertions"
)]
fn creative_packet(count: i32, item: u16, component: Option<(DataComponent, i32)>) -> Vec<u8> {
    let mut wire = vec![0, 1]; // creative inventory slot 1
    wire.write_var_int(&VarInt(count)).unwrap();
    wire.write_var_int(&VarInt(i32::from(item))).unwrap();
    wire.extend_from_slice(&[u8::from(component.is_some()), 0]);
    if let Some((component, value)) = component {
        let mut body = Vec::new();
        body.write_var_int(&VarInt(value)).unwrap();
        wire.write_var_int(&VarInt(i32::from(component.to_id())))
            .unwrap();
        wire.write_var_int(&VarInt(body.len() as i32)).unwrap();
        wire.extend(body);
    }
    wire
}

#[test]
fn creative_boundary_checks_counts_registry_and_component_values() {
    use crate::java::server::play::SSetCreativeSlot;
    let decode =
        |wire: &[u8]| SSetCreativeSlot::read(&mut &wire[..], &JavaMinecraftVersion::V_26_3);
    assert!(
        decode(&creative_packet(
            99,
            Item::STONE.id,
            Some((DataComponent::MaxStackSize, 99))
        ))
        .is_ok()
    );
    for (count, item, component) in [
        (65, Item::STONE.id, None),
        (
            1,
            Item::IRON_SWORD.id,
            Some((DataComponent::MaxStackSize, 2)),
        ),
    ] {
        assert!(decode(&creative_packet(count, item, component)).is_ok());
    }
    for (count, item, component) in [
        (100, Item::STONE.id, Some((DataComponent::MaxStackSize, 99))),
        (1, u16::MAX, None),
        (1, Item::STONE.id, Some((DataComponent::MaxStackSize, 100))),
        (1, Item::STONE.id, Some((DataComponent::MaxStackSize, 0))),
        (1, Item::IRON_SWORD.id, Some((DataComponent::MaxDamage, 0))),
        (1, Item::IRON_SWORD.id, Some((DataComponent::Damage, -1))),
    ] {
        assert!(
            decode(&creative_packet(count, item, component)).is_err(),
            "count={count}, id={item}"
        );
    }
}

#[test]
#[expect(clippy::unwrap_used, reason = "Wire regression fixture")]
fn raw_optional_counts_remain_distinct_from_the_validated_boundary() {
    // OPTIONAL_STREAM_CODEC terminates immediately for non-positive counts.
    let mut wire = Vec::new();
    wire.write_var_int(&VarInt(-1)).unwrap();
    assert!(
        ItemStackSerializer::read(&mut wire.as_slice())
            .unwrap()
            .0
            .is_empty()
    );
    assert!(
        ItemStackSerializer::read_length_prefixed_optional(&mut wire.as_slice())
            .unwrap()
            .0
            .is_empty()
    );
    // Raw codecs can carry representable counts above 99 without wrapping.
    let wire = creative_packet(100, Item::STONE.id, None);
    assert_eq!(
        ItemStackSerializer::read_length_prefixed_optional(&mut &wire[2..])
            .unwrap()
            .0
            .item_count,
        100
    );
}
#[test]
#[expect(clippy::unwrap_used, reason = "Known template wire bytes")]
fn charged_projectiles_preserve_template_bytes() {
    use pumpkin_data::data_component_impl::{ChargedProjectilesImpl, DamageImpl};
    let mut wire = vec![1]; // one template: item id, count, added, removed, patch
    wire.write_var_int(&VarInt(i32::from(Item::ARROW.id)))
        .unwrap();
    wire.extend_from_slice(&[2, 1, 0]);
    wire.write_var_int(&VarInt(i32::from(DataComponent::Damage.to_id())))
        .unwrap();
    wire.push(7);
    let component = ChargedProjectilesImpl::deserialize(&mut wire.as_slice()).unwrap();
    let projectile =
        pumpkin_data::item_stack::ItemStack::read_item_stack(&component.projectiles[0]).unwrap();
    assert_eq!(projectile.item, &Item::ARROW);
    assert_eq!(projectile.item_count, 2);
    assert_eq!(
        projectile
            .get_data_component::<DamageImpl>()
            .unwrap()
            .damage,
        7
    );
    let mut encoded = Vec::new();
    component.serialize(&mut encoded).unwrap();
    assert_eq!(encoded, wire);
}
#[test]
#[expect(clippy::unwrap_used, reason = "Known list-length wire bytes")]
fn charged_projectile_lengths_are_bounded_before_allocation() {
    use pumpkin_data::data_component_impl::ChargedProjectilesImpl;
    for length in [-1, 1025] {
        let mut wire = Vec::new();
        wire.write_var_int(&VarInt(length)).unwrap();
        assert!(ChargedProjectilesImpl::deserialize(&mut wire.as_slice()).is_err());
    }
    let mut limit = vec![0x80, 0x08]; // 1024 templates
    for _ in 0..1024 {
        limit
            .write_var_int(&VarInt(i32::from(Item::ARROW.id)))
            .unwrap();
        limit.extend_from_slice(&[1, 0, 0]);
    }
    assert_eq!(
        ChargedProjectilesImpl::deserialize(&mut limit.as_slice())
            .unwrap()
            .projectiles
            .len(),
        1024
    );
}
#[test]
#[expect(clippy::unwrap_used, reason = "Independent container-click bytes")]
fn container_click_accepts_raw_claim_counts_and_rejects_unknown_registry_ids() {
    use crate::java::server::play::SClickSlot;
    for (count, item, valid) in [
        (-1, Item::STONE.id, true),
        (0, Item::STONE.id, true),
        (99, Item::STONE.id, true),
        (100, Item::STONE.id, true),
        (1, u16::MAX, false),
    ] {
        // container id, revision, slot, button, action, zero changed slots, carried HashedStack.
        let mut wire = vec![0, 0, 0, 0, 0, 0, 0, 1];
        wire.write_var_int(&VarInt(i32::from(item))).unwrap();
        wire.write_var_int(&VarInt(count)).unwrap();
        wire.extend_from_slice(&[0, 0]); // component hashes, removals
        assert_eq!(
            SClickSlot::read(&mut wire.as_slice(), &JavaMinecraftVersion::V_26_3).is_ok(),
            valid
        );
    }
}

#[test]
fn nested_stacks_use_persistent_bounds_without_effective_stack_limits() {
    use pumpkin_data::{data_component_impl::ContainerImpl, item_stack::ItemStack};
    for (count, valid) in [(65, true), (99, true), (100, false)] {
        let mut stack = ItemStack::new(1, &Item::CHEST);
        stack.set_data_component(ContainerImpl {
            items: vec![(0, ItemStack::new(count, &Item::STONE))],
        });
        assert_eq!(
            super::item_stack_validation::validate_persistent(&stack).is_ok(),
            valid
        );
    }
}
