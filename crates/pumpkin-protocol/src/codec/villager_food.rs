use super::DataComponentCodec;
use crate::{
    VarInt,
    ser::{NetworkReadExt, NetworkWriteExt, ReadingError, WritingError},
};
use pumpkin_data::data_component_impl::VillagerFoodImpl;

impl DataComponentCodec<Self> for VillagerFoodImpl {
    // VillagerFood.STREAM_CODEC is a signed VarInt, without the persistent bound.
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(self.nutrition))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            nutrition: seq.get_var_int()?.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ServerPacket,
        codec::{data_component, item_stack_seralizer::ItemStackSerializer},
        java::server::play::SSetCreativeSlot,
    };
    use pumpkin_data::{data_component::DataComponent, item::Item, item_stack::ItemStack};
    use pumpkin_util::version::JavaMinecraftVersion;
    use std::borrow::Cow;

    const VERSION: JavaMinecraftVersion = JavaMinecraftVersion::V_26_3;

    #[test]
    fn villager_s2_signed_component_wire_literals() -> Result<(), Box<dyn std::error::Error>> {
        for (nutrition, bytes) in [
            (300, &[0xac, 0x02][..]),
            (i32::MAX, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (0, &[0x00]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
        ] {
            let value = VillagerFoodImpl { nutrition };
            let mut encoded = Vec::new();
            data_component::serialize(DataComponent::VillagerFood, &value, &mut encoded)?;
            assert_eq!(encoded, bytes);
            let mut input = bytes;
            let decoded = data_component::deserialize(DataComponent::VillagerFood, &mut input)?;
            assert!(decoded.equal(&value));
            assert!(input.is_empty());
        }
        for bytes in [
            &[0xac][..],
            &[0x80, 0x80, 0x80, 0x80, 0x80, 0],
            &[0xff, 0xff, 0xff, 0xff, 0x7f],
        ] {
            let mut input = bytes;
            assert!(data_component::deserialize(DataComponent::VillagerFood, &mut input).is_err());
        }
        Ok(())
    }

    fn raw_stack(payload: &[u8], declared_len: usize) -> Result<Vec<u8>, WritingError> {
        let mut wire = Vec::new();
        for scalar in [
            2,
            i32::from(Item::STONE.id),
            1,
            0,
            i32::from(DataComponent::VillagerFood.to_id()),
            declared_len as i32,
        ] {
            wire.write_var_int(&VarInt(scalar))?;
        }
        wire.extend_from_slice(payload);
        Ok(wire)
    }

    #[test]
    fn villager_s2_stack_framing_and_validated_creative_reader()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut stack = ItemStack::new(2, &Item::STONE);
        stack.set_data_component(VillagerFoodImpl { nutrition: 300 });
        let serializer = ItemStackSerializer(Cow::Borrowed(&stack));
        let mut normal = Vec::new();
        serializer.write_with_version(&mut normal, &VERSION)?;
        assert!(normal.ends_with(&[0xac, 0x02]));
        let decoded = ItemStackSerializer::read_with_version(&mut normal.as_slice(), &VERSION)?;
        assert!(decoded.0.are_equal(&stack));
        let mut framed = Vec::new();
        serializer.write_length_prefixed_with_version(&mut framed, &VERSION)?;
        assert_eq!(framed, raw_stack(&[0xac, 0x02], 2)?);
        assert!(
            ItemStackSerializer::read_untrusted_with_version(&mut framed.as_slice(), &VERSION)?
                .0
                .are_equal(&stack)
        );
        let mut packet = 36i16.to_be_bytes().to_vec();
        packet.extend(framed);
        let decoded = SSetCreativeSlot::read(&mut packet.as_slice(), &VERSION)?;
        assert!(decoded.clicked_item.0.are_equal(&stack));
        for payload in [&[0x00][..], &[0xff, 0xff, 0xff, 0xff, 0x0f]] {
            let raw = raw_stack(payload, payload.len())?;
            assert!(
                ItemStackSerializer::read_untrusted_with_version(&mut raw.as_slice(), &VERSION)
                    .is_ok()
            );
            let mut packet = 36i16.to_be_bytes().to_vec();
            packet.extend(raw);
            assert!(SSetCreativeSlot::read(&mut packet.as_slice(), &VERSION).is_err());
        }
        for (payload, len) in [(&[0xac][..], 1), (&[0xac, 0x02][..], 3)] {
            let mut packet = 36i16.to_be_bytes().to_vec();
            packet.extend(raw_stack(payload, len)?);
            assert!(SSetCreativeSlot::read(&mut packet.as_slice(), &VERSION).is_err());
        }
        let mut removed = ItemStack::new(3, &Item::BREAD);
        removed.remove_data_component(DataComponent::VillagerFood);
        let mut wire = Vec::new();
        ItemStackSerializer(Cow::Borrowed(&removed))
            .write_length_prefixed_with_version(&mut wire, &VERSION)?;
        let decoded =
            ItemStackSerializer::read_validated_with_version(&mut wire.as_slice(), &VERSION)?;
        assert!(decoded.0.are_equal(&removed));
        assert!(decoded.0.get_data_component::<VillagerFoodImpl>().is_none());
        Ok(())
    }

    #[test]
    fn villager_s2_validated_nested_components() -> Result<(), Box<dyn std::error::Error>> {
        use pumpkin_data::data_component_impl::{BundleContentsImpl, ContainerImpl};
        for nutrition in [7, 0, -1] {
            let mut inner = ItemStack::new(1, &Item::STONE);
            inner.set_data_component(VillagerFoodImpl { nutrition });
            let mut container = ItemStack::new(1, &Item::SHULKER_BOX);
            container.set_data_component(ContainerImpl {
                items: vec![(0, inner.clone())],
            });
            let mut bundle = ItemStack::new(1, &Item::BUNDLE);
            bundle.set_data_component(BundleContentsImpl {
                items: vec![inner],
                selected_item: -1,
            });
            for stack in [container, bundle] {
                let mut packet = 36i16.to_be_bytes().to_vec();
                ItemStackSerializer(Cow::Owned(stack))
                    .write_length_prefixed_with_version(&mut packet, &VERSION)?;
                assert_eq!(
                    SSetCreativeSlot::read(&mut packet.as_slice(), &VERSION).is_ok(),
                    nutrition > 0
                );
            }
        }
        Ok(())
    }

    #[test]
    fn villager_s2_ordinary_stack_control() -> Result<(), Box<dyn std::error::Error>> {
        for item in [&Item::WHEAT, &Item::BREAD] {
            let stack = ItemStack::new(3, item);
            let mut bytes = Vec::new();
            ItemStackSerializer(Cow::Borrowed(&stack)).write_with_version(&mut bytes, &VERSION)?;
            assert!(
                ItemStackSerializer::read_with_version(&mut bytes.as_slice(), &VERSION)?
                    .0
                    .are_equal(&stack)
            );
            let mut packet = 36i16.to_be_bytes().to_vec();
            ItemStackSerializer(Cow::Borrowed(&stack))
                .write_length_prefixed_with_version(&mut packet, &VERSION)?;
            assert!(
                SSetCreativeSlot::read(&mut packet.as_slice(), &VERSION)?
                    .clicked_item
                    .0
                    .are_equal(&stack)
            );
        }
        Ok(())
    }
}
