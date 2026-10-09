use super::CRegistryData;
use crate::{ClientPacket, ser::NetworkReadExt};
use pumpkin_data::{biome::Biome, registry::Registry};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::version::JavaMinecraftVersion;

fn compare_compound(actual: &NbtCompound, expected: &NbtCompound, path: &str) {
    assert_eq!(actual.child_tags.len(), expected.child_tags.len(), "{path}");
    for (key, expected) in &expected.child_tags {
        let actual = actual.get(key);
        assert!(actual.is_some(), "missing {path}/{key}");
        if let Some(actual) = actual {
            compare_tag(actual, expected, &format!("{path}/{key}"));
        }
    }
}

fn compare_tag(actual: &NbtTag, expected: &NbtTag, path: &str) {
    match (actual, expected) {
        (NbtTag::Compound(actual), NbtTag::Compound(expected)) => {
            compare_compound(actual, expected, path);
        }
        (NbtTag::List(actual), NbtTag::List(expected)) => {
            assert_eq!(actual.len(), expected.len(), "{path}");
            for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                compare_tag(actual, expected, &format!("{path}/{index}"));
            }
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}

#[test]
fn biome_configuration_packet_matches_vanilla_26_3() -> Result<(), Box<dyn std::error::Error>> {
    // RegistrySynchronization.packRegistries uses Biome.NETWORK_CODEC, not DIRECT_CODEC.
    let mut expected =
        include_bytes!("../../../../../../assets/tests/vanilla_biome_registry_26_3.bin").as_slice();
    let version = JavaMinecraftVersion::V_26_3;
    let registries = Registry::get_synced(version);
    let registry = registries
        .iter()
        .find(|registry| registry.registry_id == "minecraft:worldgen/biome")
        .ok_or("missing biome registry")?;
    let mut bytes = Vec::new();
    CRegistryData::new(&registry.registry_id, &registry.registry_entries)
        .write_packet_data(&mut bytes, &version)?;
    let mut packet = bytes.as_slice();
    assert_eq!(packet.get_str()?, expected.get_str()?);
    let count = expected.get_var_int()?.0;
    assert_eq!(packet.get_var_int()?.0, count);
    for id in 0..count {
        let name = expected.get_str()?;
        assert_eq!(packet.get_str()?, name);
        assert_eq!(
            Biome::from_name(
                name.strip_prefix("minecraft:")
                    .ok_or("invalid biome name")?
            )
            .map(|biome| i32::from(biome.id)),
            Some(id)
        );
        assert!(expected.get_bool()?);
        assert!(packet.get_bool()?);
        let data = packet
            .get_compound_nbt_with_version(&version)?
            .ok_or("missing biome NBT")?;
        let expected = expected
            .get_compound_nbt_with_version(&version)?
            .ok_or("missing vanilla biome NBT")?;
        compare_compound(&data, &expected, &name);
    }
    assert!(packet.is_empty());
    assert!(expected.is_empty());
    Ok(())
}
