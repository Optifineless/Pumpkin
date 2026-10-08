use super::*;
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_nbt::compound::NbtCompound;

// Handwritten payloads from vanilla 26.3 Filterable/BookContent STREAM_CODEC.
#[test]
fn book_stream_fixtures() -> Result<(), Box<dyn std::error::Error>> {
    let writable = WritableBookContentImpl {
        pages: vec!["hi".into()],
    };
    let mut bytes = Vec::new();
    writable.serialize(&mut bytes)?;
    assert_eq!(bytes, [1, 2, b'h', b'i', 0]);
    let decoded = WritableBookContentImpl::deserialize(&mut bytes.as_slice())?;
    assert_eq!(decoded, writable);

    let written = WrittenBookContentImpl {
        title: "T".into(),
        author: "A".into(),
        pages: vec![pumpkin_util::text::TextComponent::text("hi")],
    };
    bytes.clear();
    written.serialize(&mut bytes)?;
    // title + optional filtered, author, generation, pages, unnamed string NBT,
    // optional filtered page, resolved.
    assert_eq!(
        bytes,
        [1, b'T', 0, 1, b'A', 0, 1, 8, 0, 2, b'h', b'i', 0, 1]
    );
    let mut input = bytes.as_slice();
    assert_eq!(WrittenBookContentImpl::deserialize(&mut input)?, written);
    assert!(input.is_empty());
    Ok(())
}

#[test]
fn horn_holder_fixture() -> Result<(), Box<dyn std::error::Error>> {
    // InstrumentComponent.STREAM_CODEC -> ByteBufCodecs.holder: reference id + 1.
    let horn = InstrumentImpl::read_data(&NbtTag::String("minecraft:sing_goat_horn".into()))
        .ok_or("missing instrument")?;
    let mut bytes = Vec::new();
    horn.serialize(&mut bytes)?;
    assert_eq!(bytes, [7]);
    let mut input = bytes.as_slice();
    let decoded = InstrumentImpl::deserialize(&mut input)?;
    assert_eq!(
        decoded.write_data(),
        NbtTag::String("minecraft:sing_goat_horn".into())
    );
    assert!(input.is_empty());
    Ok(())
}

#[test]
fn painting_holder_fixture() -> Result<(), Box<dyn std::error::Error>> {
    // PaintingVariant.STREAM_CODEC uses the same holder marker, never a string.
    let painting = PaintingVariantImpl {
        value: Cow::Borrowed("minecraft:alban"),
    };
    let mut bytes = Vec::new();
    painting.serialize(&mut bytes)?;
    assert_eq!(bytes, [1]);
    let mut input = bytes.as_slice();
    assert_eq!(PaintingVariantImpl::deserialize(&mut input)?, painting);
    assert!(input.is_empty());
    Ok(())
}

#[test]
fn custom_data_uses_unnamed_compound_fixture() -> Result<(), Box<dyn std::error::Error>> {
    // CustomData.STREAM_CODEC uses COMPOUND_TAG: type, entries, end (no root name).
    let mut data = NbtCompound::new();
    data.put_bool("ok", true);
    let component = CustomDataImpl::new(data);
    let mut bytes = Vec::new();
    component.serialize(&mut bytes)?;
    assert_eq!(bytes, [10, 1, 0, 2, b'o', b'k', 1, 0]);
    let mut input = bytes.as_slice();
    assert_eq!(CustomDataImpl::deserialize(&mut input)?, component);
    assert!(input.is_empty());
    Ok(())
}

#[test]
fn charged_crossbow_template_fixture() -> Result<(), Box<dyn std::error::Error>> {
    // ChargedProjectiles.STREAM_CODEC is a list of ItemStackTemplate.STREAM_CODEC.
    let mut arrow = ItemStack::new(1, &Item::ARROW);
    arrow.set_data_component(IntangibleProjectileImpl);
    let mut projectile = NbtCompound::new();
    arrow.write_item_stack(&mut projectile);
    let charged = ChargedProjectilesImpl {
        projectiles: vec![projectile],
    };
    let mut bytes = Vec::new();
    charged.serialize(&mut bytes)?;
    let mut expected = vec![1];
    VarInt(i32::from(Item::ARROW.id)).encode(&mut expected)?;
    expected.extend_from_slice(&[1, 1, 0, DataComponent::IntangibleProjectile.to_id(), 10, 0]);
    assert_eq!(bytes, expected);
    let mut input = bytes.as_slice();
    let decoded = ChargedProjectilesImpl::deserialize(&mut input)?;
    let stack = ItemStack::read_item_stack(&decoded.projectiles[0]).ok_or("invalid projectile")?;
    assert_eq!(stack.item, &Item::ARROW);
    assert!(
        stack
            .get_data_component::<IntangibleProjectileImpl>()
            .is_some()
    );
    assert!(input.is_empty());
    Ok(())
}

#[test]
fn vanilla_saved_projectile_without_count_fixture() -> Result<(), Box<dyn std::error::Error>> {
    // ItemStackTemplate.MAP_CODEC omits count=1: [{id:"minecraft:arrow"}].
    let mut projectile = NbtCompound::new();
    projectile.put_string("id", "minecraft:arrow".into());
    let charged = ChargedProjectilesImpl {
        projectiles: vec![projectile],
    };
    let mut bytes = Vec::new();
    charged.serialize(&mut bytes)?;
    let mut expected = vec![1];
    VarInt(i32::from(Item::ARROW.id)).encode(&mut expected)?;
    expected.extend_from_slice(&[1, 0, 0]);
    assert_eq!(bytes, expected);
    Ok(())
}

#[test]
fn empty_charged_projectile_templates_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
    for (item, count) in [(&Item::AIR, 1), (&Item::ARROW, 0)] {
        let mut bytes = vec![1];
        VarInt(i32::from(item.id)).encode(&mut bytes)?;
        bytes.extend_from_slice(&[count, 0, 0]);
        let error = ChargedProjectilesImpl::deserialize(&mut bytes.as_slice()).unwrap_err();
        assert!(error.to_string().contains("Item must be non-empty"));
    }
    Ok(())
}

#[test]
fn invalid_loaded_components_do_not_abort_container_content()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::{
        ClientPacket, codec::item_stack_seralizer::ItemStackSerializer,
        java::client::play::CSetContainerContent,
    };
    // Inventory.read_nbt leaves rejected stacks empty; the remaining slots must still sync.
    let mut book = NbtCompound::new();
    book.put_string("title", "x".repeat(33));
    book.put_string("author", "A".into());
    let invalid = [
        (
            &Item::WRITTEN_BOOK,
            DataComponent::WrittenBookContent,
            NbtTag::Compound(book),
        ),
        (
            &Item::PAINTING,
            DataComponent::PaintingVariant,
            NbtTag::String("minecraft:missing".into()),
        ),
    ];
    let mut slots = Vec::new();
    for (item, component, data) in invalid {
        let mut components = NbtCompound::new();
        components.put(component.to_name(), data);
        let mut nbt = NbtCompound::new();
        nbt.put_string("id", format!("minecraft:{}", item.registry_key));
        nbt.put_int("count", 1);
        nbt.put_compound("components", components);
        slots.push(ItemStackSerializer(Cow::Owned(
            ItemStack::read_item_stack(&nbt).unwrap_or_else(|| ItemStack::EMPTY.clone()),
        )));
    }
    slots.push(ItemStackSerializer(Cow::Owned(ItemStack::new(
        1,
        &Item::STICK,
    ))));
    let carried = ItemStackSerializer(Cow::Borrowed(ItemStack::EMPTY));
    let packet = CSetContainerContent::new(VarInt(0), VarInt(1), &slots, &carried);
    let mut bytes = Vec::new();
    packet.write_packet_data(&mut bytes, &JavaMinecraftVersion::V_26_3)?;
    let mut expected = vec![0, 1, 3, 0, 0, 1];
    VarInt(i32::from(Item::STICK.id)).encode(&mut expected)?;
    expected.extend_from_slice(&[0, 0, 0]);
    assert_eq!(bytes, expected);
    Ok(())
}

#[test]
fn invalid_component_inputs_are_rejected() {
    let book = WritableBookContentImpl {
        pages: vec![String::new(); 101],
    };
    assert!(book.serialize(&mut Vec::new()).is_err());
    assert!(WritableBookContentImpl::deserialize(&mut [101].as_slice()).is_err());
    assert!(
        ChargedProjectilesImpl::deserialize(&mut [0xff, 0xff, 0xff, 0xff, 0x0f].as_slice())
            .is_err()
    );
}

#[test]
fn book_string_bounds_count_java_characters() -> Result<(), Box<dyn std::error::Error>> {
    // Utf8String.write permits 32 UTF-16 title characters, even with multibyte UTF-8.
    let mut written = WrittenBookContentImpl {
        title: "é".repeat(32),
        author: "A".into(),
        pages: Vec::new(),
    };
    let mut bytes = Vec::new();
    written.serialize(&mut bytes)?;
    assert_eq!(bytes[0], 64);
    assert_eq!(&bytes[1..65], written.title.as_bytes());
    assert_eq!(
        WrittenBookContentImpl::deserialize(&mut bytes.as_slice())?,
        written
    );
    written.title.push('é');
    assert!(written.serialize(&mut Vec::new()).is_err());
    written.title = "😀".repeat(17);
    assert!(written.serialize(&mut Vec::new()).is_err());
    Ok(())
}

#[test]
fn inline_instrument_fixture() -> Result<(), Box<dyn std::error::Error>> {
    // Instrument.DIRECT_STREAM_CODEC: sound holder, floats, damage VarInt, component NBT.
    let instrument = InstrumentImpl::Direct {
        sound_event: IdOr::Value(SoundEvent {
            sound_name: Cow::Borrowed("x:y"),
            range: None,
        }),
        use_duration: 1.0,
        range: 2.0,
        durability_damage: 3,
        description: pumpkin_util::text::TextComponent::text("D"),
    };
    let expected = [
        0, 0, 3, b'x', b':', b'y', 0, 0x3f, 0x80, 0, 0, 0x40, 0, 0, 0, 3, 8, 0, 1, b'D',
    ];
    let mut bytes = Vec::new();
    instrument.serialize(&mut bytes)?;
    assert_eq!(bytes, expected);
    let mut input = expected.as_slice();
    let decoded = InstrumentImpl::deserialize(&mut input)?;
    assert_eq!(decoded, instrument);
    assert!(input.is_empty());
    let nbt = instrument.write_data();
    assert_eq!(
        InstrumentImpl::read_data(&nbt).ok_or("invalid inline instrument")?,
        instrument
    );
    Ok(())
}

#[test]
fn container_content_with_horn_painting_and_custom_data_fixture()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::{
        ClientPacket, codec::item_stack_seralizer::ItemStackSerializer,
        java::client::play::CSetContainerContent,
    };
    let mut horn = ItemStack::new(1, &Item::GOAT_HORN);
    horn.set_data_component(
        InstrumentImpl::read_data(&NbtTag::String("minecraft:sing_goat_horn".into()))
            .ok_or("invalid horn")?,
    );
    let mut painting = ItemStack::new(1, &Item::PAINTING);
    painting.set_data_component(PaintingVariantImpl {
        value: Cow::Borrowed("minecraft:alban"),
    });
    let mut data = NbtCompound::new();
    data.put_bool("ok", true);
    let mut stick = ItemStack::new(1, &Item::STICK);
    stick.set_data_component(CustomDataImpl::new(data));
    let slots = [horn, painting, stick].map(|stack| ItemStackSerializer(Cow::Owned(stack)));
    let carried = ItemStackSerializer(Cow::Borrowed(ItemStack::EMPTY));
    let packet = CSetContainerContent::new(VarInt(0), VarInt(300), &slots, &carried);
    let mut bytes = Vec::new();
    packet.write_packet_data(&mut bytes, &JavaMinecraftVersion::V_26_3)?;
    // ClientboundContainerSetContentPacket -> CONTAINER_ID, state, OPTIONAL_LIST_STREAM_CODEC, carried.
    let mut expected = vec![0, 0xac, 0x02, 3];
    for (item, component, payload) in [
        (&Item::GOAT_HORN, DataComponent::Instrument, vec![7]),
        (&Item::PAINTING, DataComponent::PaintingVariant, vec![1]),
        (
            &Item::STICK,
            DataComponent::CustomData,
            vec![10, 1, 0, 2, b'o', b'k', 1, 0],
        ),
    ] {
        expected.push(1);
        VarInt(i32::from(item.id)).encode(&mut expected)?;
        expected.extend_from_slice(&[1, 0, component.to_id()]);
        expected.extend(payload);
    }
    expected.push(0);
    assert_eq!(bytes, expected);
    let mut input = bytes.as_slice();
    assert_eq!(input.get_var_int()?, VarInt(0));
    assert_eq!(input.get_var_int()?, VarInt(300));
    assert_eq!(input.get_var_int()?, VarInt(3));
    for slot in slots {
        let decoded =
            ItemStackSerializer::read_with_version(&mut input, &JavaMinecraftVersion::V_26_3)?;
        assert!(decoded.to_stack().are_equal(&slot.to_stack()));
    }
    assert_eq!(input.get_var_int()?, VarInt(0));
    assert!(input.is_empty());
    Ok(())
}

#[test]
fn add_entity_matches_vanilla_zero_motion_fixture() -> Result<(), Box<dyn std::error::Error>> {
    use crate::{ClientPacket, java::client::play::CSpawnEntity};
    use pumpkin_util::math::vector3::Vector3;
    // ClientboundAddEntityPacket.write: UUID, entity registry, doubles, LP vector, angles, data.
    let packet = CSpawnEntity::new_packed(
        VarInt(300),
        uuid::Uuid::from_u128(1),
        VarInt(i32::from(EntityType::COW.id)),
        Vector3::new(1.0, 2.0, -0.5),
        0x20,
        0x40,
        0x60,
        VarInt(300),
        Vector3::default(),
    );
    let mut bytes = Vec::new();
    packet.write_packet_data(&mut bytes, &JavaMinecraftVersion::V_26_3)?;
    let mut expected = vec![0xac, 0x02];
    expected.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    VarInt(i32::from(EntityType::COW.id)).encode(&mut expected)?;
    expected.extend_from_slice(&[
        0x3f, 0xf0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0, 0xbf, 0xe0, 0, 0, 0, 0, 0, 0, 0,
        0x20, 0x40, 0x60, 0xac, 0x02,
    ]);
    assert_eq!(bytes, expected);
    let mut input = bytes.as_slice();
    assert_eq!(input.get_var_int()?, VarInt(300));
    assert_eq!(input.get_uuid()?, uuid::Uuid::from_u128(1));
    assert_eq!(input.get_var_int()?.0, i32::from(EntityType::COW.id));
    assert_eq!(
        [
            input.get_f64_be()?,
            input.get_f64_be()?,
            input.get_f64_be()?
        ],
        [1.0, 2.0, -0.5]
    );
    assert_eq!(input.get_u8()?, 0); // LpVec3.read consumes ONE byte for a zero vector.
    assert_eq!(
        [input.get_u8()?, input.get_u8()?, input.get_u8()?],
        [0x20, 0x40, 0x60]
    );
    assert_eq!(input.get_var_int()?, VarInt(300));
    assert!(input.is_empty());
    Ok(())
}

#[test]
fn custom_data_survives_untrusted_inventory_component_fixture()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::codec::item_stack_seralizer::ItemStackSerializer;
    // DataComponentPatch.DELIMITED_STREAM_CODEC, used by creative/inventory input.
    let mut bytes = vec![1];
    VarInt(i32::from(Item::STICK.id)).encode(&mut bytes)?;
    bytes.extend_from_slice(&[
        1,
        0,
        DataComponent::CustomData.to_id(),
        8,
        10,
        1,
        0,
        2,
        b'o',
        b'k',
        1,
        0,
    ]);
    let mut input = bytes.as_slice();
    let stack = ItemStackSerializer::read_untrusted_with_version(
        &mut input,
        &JavaMinecraftVersion::V_26_3,
    )?
    .to_stack();
    assert_eq!(stack.item, &Item::STICK);
    assert_eq!(
        stack
            .get_data_component::<CustomDataImpl>()
            .ok_or("missing custom data")?
            .data
            .get_bool("ok"),
        Some(true)
    );
    assert!(input.is_empty());
    Ok(())
}
