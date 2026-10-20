use super::*;
use crate::{
    bedrock::network_item::{ItemStackWrapper, NetworkItemDescriptor, NetworkItemStackDescriptor},
    codec::item_stack_seralizer::ItemStackSerializer,
    serial::{PacketRead, PacketWrite},
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::version::JavaMinecraftVersion;

#[test]
fn jukebox_registered_holder_wire_fixture() -> Result<(), Box<dyn std::error::Error>> {
    // Vanilla 26.3 JukeboxSong.STREAM_CODEC: cat is registry entry 5, holder 6.
    for song in ["cat", "minecraft:cat"] {
        let mut bytes = Vec::new();
        JukeboxPlayableImpl { song }.serialize(&mut bytes)?;
        assert_eq!(bytes, [6]);
    }
    let mut bytes = Vec::new();
    assert!(
        JukeboxPlayableImpl { song: "" }
            .serialize(&mut bytes)
            .is_err()
    );
    assert!(bytes.is_empty(), "never write a bare inline-holder marker");
    Ok(())
}

#[test]
fn jukebox_incoming_registered_holder_wire_fixture() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = [6, 0xff].as_slice();
    assert_eq!(JukeboxPlayableImpl::deserialize(&mut input)?.song, "cat");
    assert_eq!(input, [0xff], "holder must leave the metadata terminator");

    // Inline songs have no representation in the retained component type.
    for bytes in [&[0][..], &[23], &[0xff, 0xff, 0xff, 0xff, 0x0f], &[0x80]] {
        let mut input = bytes;
        assert!(JukeboxPlayableImpl::deserialize(&mut input).is_err());
    }
    Ok(())
}

#[test]
fn jukebox_stack_writers_preserve_holder_boundaries() -> Result<(), Box<dyn std::error::Error>> {
    let version = JavaMinecraftVersion::V_26_3;
    for song in [
        "cat",
        "minecraft:cat",
        "",
        "minecraft:not_a_song",
        "other:cat",
    ] {
        let mut stack = ItemStack::new(1, &Item::MUSIC_DISC_13);
        stack.set_data_component(JukeboxPlayableImpl { song });
        stack.remove_data_component(DataComponent::Damage);
        let mut saved = NbtCompound::new();
        stack.write_item_stack(&mut saved);
        let serializer = ItemStackSerializer(std::borrow::Cow::Borrowed(&stack));
        let registered = matches!(song, "cat" | "minecraft:cat");
        for route in 0..5 {
            let mut bytes = Vec::new();
            match route {
                0 => serializer.write_with_version(&mut bytes, &version)?,
                1 => serializer.write_untrusted_with_version(&mut bytes, &version)?,
                2 => serializer.write_template_with_version(&mut bytes, &version)?,
                3 => serializer.write_item_cost_with_version(&mut bytes, &version)?,
                _ => serializer.write_optional_template_with_version(&mut bytes, &version)?,
            }
            let mut expected = Vec::new();
            if route == 4 {
                expected.write_bool(true)?;
            }
            let item_id = VarInt::from(Item::MUSIC_DISC_13.id);
            let count = VarInt(1);
            let (first, second) = if route >= 2 {
                (item_id, count)
            } else {
                (count, item_id)
            };
            expected.write_var_int(&first)?;
            expected.write_var_int(&second)?;
            expected.write_var_int(&VarInt(i32::from(registered)))?;
            if route != 3 {
                expected.write_var_int(&VarInt(1))?;
            }
            if registered {
                expected.write_var_int(&VarInt::from(DataComponent::JukeboxPlayable.to_id()))?;
                if route == 1 {
                    expected.push(1); // length-prefixed holder payload
                }
                expected.push(6);
            }
            if route != 3 {
                expected.write_var_int(&VarInt::from(DataComponent::Damage.to_id()))?;
            }
            assert_eq!(bytes, expected, "song={song:?}, writer={route}");
            if route == 0 {
                let decoded =
                    ItemStackSerializer::read_with_version(&mut bytes.as_slice(), &version)?
                        .to_stack();
                assert_eq!(
                    decoded
                        .get_data_component::<JukeboxPlayableImpl>()
                        .map(|p| p.song),
                    Some(if registered { "cat" } else { "minecraft:13" })
                );
            }
        }
        let mut after = NbtCompound::new();
        stack.write_item_stack(&mut after);
        assert_eq!(after, saved, "client filtering must not change saved data");
    }
    Ok(())
}

#[test]
fn jukebox_bedrock_items_remain_decodable() -> Result<(), Box<dyn std::error::Error>> {
    for song in ["cat", ""] {
        let mut stack = ItemStack::new(1, &Item::MUSIC_DISC_13);
        stack.set_data_component(JukeboxPlayableImpl { song });
        let mut bytes = Vec::new();
        ItemStackWrapper::from(&stack).write(&mut bytes)?;
        let mut input = bytes.as_slice();
        let decoded = ItemStackWrapper::read(&mut input)?;
        assert_ne!(decoded.id, 0);
        assert_eq!(decoded.stack_size, 1);
        assert_eq!(decoded.net_id, Some(stack.uid));
        assert!(decoded.nbt_data.is_empty());
        assert!(input.is_empty());

        bytes.clear();
        NetworkItemDescriptor::from(&stack).write(&mut bytes)?;
        let mut input = bytes.as_slice();
        let decoded = NetworkItemDescriptor::read(&mut input)?;
        assert_ne!(decoded.id.0, 0);
        assert_eq!(decoded.stack_size, 1);
        assert!(decoded.nbt_data.is_empty());
        assert!(input.is_empty());

        bytes.clear();
        NetworkItemStackDescriptor::from(&stack).write(&mut bytes)?;
        let mut input = bytes.as_slice();
        let decoded = NetworkItemStackDescriptor::read(&mut input)?;
        assert_ne!(decoded.id, 0);
        assert_eq!(decoded.stack_size, 1);
        assert_eq!(decoded.net_id, Some(stack.uid));
        assert_eq!(decoded.extra_data, [0; 10]);
        assert!(input.is_empty());
    }
    Ok(())
}
