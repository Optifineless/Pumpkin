use super::*;
use crate::{
    entity::r#type::from_type,
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::entity::EntityType;
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_util::{math::vector3::Vector3, version::JavaMinecraftVersion};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review3_name_tag_entity_save_load_keeps_numeric_translation_and_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let server = server(directory.path());
    let world = world(&server, directory.path());
    let player = TestPlayer::new(&world).player;
    for (key, fallback, arguments, rendered) in [
        ("unknown.key", "Key %s", vec![NbtTag::Int(1)], "Key 1"),
        ("unknown.key", "Vault", vec![], "Vault"),
    ] {
        let mut nbt = NbtCompound::new();
        nbt.put_string("translate", key.into());
        nbt.put_string("fallback", fallback.into());
        if !arguments.is_empty() {
            nbt.put_list("with", arguments);
        }
        let tag = NbtTag::Compound(nbt);
        let mut name_tag = ItemStack::new(1, &Item::NAME_TAG);
        name_tag.set_data_component(CustomNameImpl::read_data(&tag).unwrap());
        let entity = from_type(
            &EntityType::COW,
            Vector3::default(),
            &world,
            uuid::Uuid::new_v4(),
        );
        NameTagItem.use_on_entity(&mut name_tag, &player, entity.clone());
        assert!(name_tag.is_empty());
        let mut saved = NbtCompound::new();
        entity.write_nbt(&mut saved);
        assert_eq!(saved.get("CustomName"), Some(&tag));
        let bytes = pumpkin_nbt::Nbt::new(String::new(), saved).write();
        let saved = pumpkin_nbt::Nbt::read(&mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(
            &mut std::io::Cursor::new(bytes.as_ref()),
        ))
        .unwrap();
        let loaded = from_type(
            &EntityType::COW,
            Vector3::default(),
            &world,
            uuid::Uuid::new_v4(),
        );
        loaded.read_nbt_non_mut(&saved.root_tag);
        let name = loaded.get_entity().custom_name.load();
        let name = name.as_ref().as_ref().unwrap();
        assert_eq!(
            name.0.to_nbt_tag_for_version(&JavaMinecraftVersion::V_26_3),
            tag
        );
        assert_eq!(name.clone().get_text(), rendered);
    }
    assert!(world.level.shutdown().await.is_ok());
}
